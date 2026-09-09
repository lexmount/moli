use super::*;

impl ScriptVm {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn snapshot_globals(&mut self) -> Result<Option<BTreeMap<String, JsValueSnapshot>>> {
        let baseline_json = serde_json::to_string(&self.baseline_globals)
            .context("failed to encode baseline globals for script snapshot")?;
        let snapshot_source = format!(
            r#"
(() => {{
  const baseline = Object.create(null);
  for (const name of {baseline_json}) {{
    baseline[name] = true;
  }}
  const out = {{}};
  const describeFiniteNumberFallback = (value) => {{
    if (Number.isNaN(value)) {{
      return "NaN";
    }}
    if (value === Infinity) {{
      return "Infinity";
    }}
    if (value === -Infinity) {{
      return "-Infinity";
    }}
    return "number";
  }};
  const describeUnsupported = (value) => {{
    switch (typeof value) {{
      case "bigint":
        return "bigint";
      case "function":
        return "[function]";
      case "object": {{
        try {{
          return Array.isArray(value) ? "[array]" : "[object]";
        }} catch (_error) {{
          // Array.isArray throws for a revoked Proxy. Snapshotting is
          // best-effort observation and must not fail the whole page merely
          // because an unsupported object can no longer be inspected.
          return "[object]";
        }}
      }}
      case "symbol":
        return "symbol";
      default:
        return "<unsupported>";
    }}
  }};
  const isArrayIndexName = (name) => {{
    const index = name >>> 0;
    return index !== 4294967295 && String(index) === name;
  }};
  for (const name of Object.getOwnPropertyNames(globalThis)) {{
    if (baseline[name] === true || isArrayIndexName(name)) {{
      continue;
    }}

    const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
    if (!descriptor || !("value" in descriptor)) {{
      out[name] = {{ kind: "unsupported", value: "[accessor]" }};
      continue;
    }}

    const value = descriptor.value;
    if (value === undefined) {{
      out[name] = {{ kind: "undefined" }};
      continue;
    }}

    if (value === null) {{
      out[name] = {{ kind: "null" }};
      continue;
    }}

    switch (typeof value) {{
      case "boolean":
        out[name] = {{ kind: "boolean", value }};
        break;
      case "number":
        out[name] = Number.isFinite(value)
          ? {{ kind: "number", value }}
          : {{ kind: "unsupported", value: describeFiniteNumberFallback(value) }};
        break;
      case "string":
        out[name] = {{ kind: "string", value }};
        break;
      default:
        out[name] = {{ kind: "unsupported", value: describeUnsupported(value) }};
        break;
    }}
  }}
  return JSON.stringify(out);
}})()
"#
        );
        let default_context = &self.page_default_runtime.context as *const _;
        let snapshot_json = self
            .eval_string_in_context_ptr_internal_snapshot(default_context, &snapshot_source)
            .context("failed to evaluate script state snapshot")?;
        let snapshot = serde_json::from_str::<BTreeMap<String, SerializedJsValue>>(&snapshot_json)
            .context("failed to deserialize script state snapshot")?;

        Ok(Some(
            snapshot
                .into_iter()
                .map(|(name, value)| (name, value.into_snapshot()))
                .collect(),
        ))
    }

    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn snapshot_globals(&mut self) -> Result<Option<BTreeMap<String, JsValueSnapshot>>> {
        let _ = &self.baseline_globals;
        Ok(None)
    }

    pub(crate) fn take_runtime_observable_report_output(
        &mut self,
    ) -> Result<ScriptObservableOutput> {
        self.sync_runtime_observable_source_events()?;
        Ok(self
            .runtime_observable_source_queue
            .take_report_observable_output(
                self.runtime_observable_default_execution_context_id(),
                self.page_default_runtime.runtime_observable_context_token,
            ))
    }

    pub(crate) fn snapshot_console_messages_with_context(
        &mut self,
    ) -> Result<Vec<RuntimeConsoleMessageSnapshot>> {
        let contexts = self.page_runtime_observable_contexts();
        let mut messages = Vec::new();
        for context in contexts {
            let Some(execution_context_id) = context.execution_context_id else {
                continue;
            };
            let mut context_messages =
                self.snapshot_console_message_details_in_context(context.context)?;
            for mut message in context_messages.drain(..) {
                if let Some(object) = message.as_object_mut() {
                    object.insert("executionContextId".to_owned(), json!(execution_context_id));
                }
                messages.push(
                    serde_json::from_value::<RuntimeConsoleMessageSnapshot>(message)
                        .context("runtime console message snapshot has invalid shape")?,
                );
            }
        }
        Ok(messages)
    }

    pub(super) fn page_runtime_observable_contexts(&self) -> Vec<PageRuntimeObservableContext> {
        let mut contexts = vec![PageRuntimeObservableContext {
            execution_context_id: self.runtime_observable_default_execution_context_id(),
            context_token: self.page_default_runtime.runtime_observable_context_token,
            context: &self.page_default_runtime.context as *const _,
        }];
        let mut isolated_context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids()
            .collect::<Vec<_>>();
        isolated_context_ids.sort_unstable();
        for execution_context_id in isolated_context_ids {
            if let Some(world) = self
                .page_isolated_world_contexts
                .context(execution_context_id)
            {
                contexts.push(PageRuntimeObservableContext {
                    execution_context_id: Some(execution_context_id),
                    context_token: world.runtime_observable_context_token,
                    context: &world.context as *const _,
                });
            }
        }
        let mut child_default_context_ids = self
            .child_frame_realm_store
            .execution_context_ids()
            .collect::<Vec<_>>();
        child_default_context_ids.sort_unstable();
        for execution_context_id in child_default_context_ids {
            if let Some(world) = self.child_frame_realm_store.get(&execution_context_id) {
                contexts.push(PageRuntimeObservableContext {
                    execution_context_id: Some(execution_context_id),
                    context_token: world.runtime_observable_context_token,
                    context: &world.context as *const _,
                });
            }
        }
        contexts
    }

    pub(super) fn sync_runtime_observable_source_events(&mut self) -> Result<()> {
        let contexts = self.page_runtime_observable_contexts();
        let active_tokens = contexts
            .iter()
            .map(|context| context.context_token)
            .collect::<BTreeSet<_>>();
        let token_to_execution_context_id = contexts
            .iter()
            .filter_map(|context| {
                context
                    .execution_context_id
                    .map(|execution_context_id| (context.context_token, execution_context_id))
            })
            .collect::<BTreeMap<_, _>>();
        let active_contexts = contexts
            .iter()
            .filter_map(|context| context.execution_context_id)
            .collect::<BTreeSet<_>>();
        let mut host = self._context_host.borrow_mut();
        let pending_console_events = host.take_pending_runtime_observable_console_source_events();
        drop(host);
        self.runtime_observable_source_queue.sync_source_events(
            &active_contexts,
            &active_tokens,
            &token_to_execution_context_id,
            pending_console_events,
        );
        Ok(())
    }

    pub(crate) fn settle_renderer_output_publication(
        &mut self,
    ) -> Option<crate::runtime::RendererOutputPublication> {
        self.page_inspector
            .devtools_target()
            .pause_ref()
            .finish_owner_turn();
        let isolate = self.renderer_document_isolate.clone();
        isolate.with_renderer_document_isolate_and_inspector_mut(|_, _| {
            self.settle_renderer_output_prefix()
        })
    }

    /// Resolve a nested handler's prefix while its enclosing V8 turn remains
    /// paused. The caller must already be on the entered isolate (ordinary
    /// settlement enters above; nested Main runs within V8's pause loop).
    /// Do not borrow the suspended isolate holder or clear its resume/step
    /// transition while reading session state.
    pub(crate) fn settle_renderer_output_prefix(
        &mut self,
    ) -> Option<crate::runtime::RendererOutputPublication> {
        self.sync_runtime_observable_source_events()
            .expect("runtime observable source synchronization should be infallible");
        let environment = self.renderer_page_script_environment.as_ref()?;
        let output_journal = environment.output_journal();
        let mut pending = output_journal.take_pending_for_resolution()?;

        let current_agent_token = self.page_inspector.agent_token();
        for record in pending.records_mut() {
            record.with_runtime_inspector_batch_mut(|batch| {
                let raw_messages = batch
                    .messages
                    .iter()
                    .cloned()
                    .map(RendererRuntimeInspectorMessage::into_v8_inspector_message)
                    .collect::<Vec<_>>();
                self.page_inspector
                    .record_execution_context_state(&raw_messages, self.root_frame_id.as_deref());
                self.page_isolated_world_contexts
                    .record_inspector_context_state(&raw_messages, self.root_frame_id.as_deref());
                if batch.agent_token == current_agent_token {
                    batch.v8_state_update = self
                        .page_inspector
                        .v8_session_state(batch.session.wire_session_id());
                }
            });
        }
        Some(pending.finish())
    }

    pub(crate) fn append_live_command_output_prefix(&self) {
        self._context_host.borrow().append_live_turn_output_prefix();
    }

    pub(crate) fn append_renderer_output_records(
        &self,
        records: impl IntoIterator<Item = crate::runtime::PendingRendererOutputRecord>,
    ) {
        let environment = self
            .renderer_page_script_environment
            .as_ref()
            .expect("a live Page command must have a renderer output journal");
        environment.output_journal().append_records(records);
    }

    pub(crate) fn append_renderer_command_output_records(
        &self,
        records: Vec<crate::runtime::PendingRendererOutputRecord>,
    ) -> crate::runtime::RendererOutputCursor {
        let environment = self
            .renderer_page_script_environment
            .as_ref()
            .expect("a live Page command must have a renderer output journal");
        environment.output_journal().append_command_records(records)
    }

    pub(crate) fn has_renderer_output_journal(&self) -> bool {
        self.renderer_page_script_environment.is_some()
    }

    pub(crate) fn renderer_command_output_journal(
        &self,
    ) -> crate::runtime::RendererTurnOutputJournal {
        self.renderer_page_script_environment
            .as_ref()
            .expect("a live native frontend command requires a Page output journal")
            .output_journal()
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn bind_renderer_output_journal_for_test(
        &mut self,
        output_journal: crate::runtime::RendererTurnOutputJournal,
    ) {
        self._context_host
            .borrow_mut()
            .bind_output_journal(output_journal);
    }

    pub(crate) fn renderer_output_tail_cursor(
        &self,
    ) -> Option<crate::runtime::RendererOutputCursor> {
        self.renderer_page_script_environment
            .as_ref()
            .and_then(|environment| environment.output_journal().last_published_cursor())
    }

    pub(crate) fn declare_renderer_output_fence(
        &self,
        cursor: crate::runtime::RendererOutputCursor,
    ) -> crate::runtime::RendererOutputFence {
        self.renderer_page_script_environment
            .as_ref()
            .expect("a renderer output fence requires a Page script environment")
            .output_journal()
            .declare_fence(cursor)
    }

    pub(crate) fn renderer_document_isolate_ops(
        &mut self,
    ) -> ScriptVmRendererDocumentIsolateOps<'_> {
        ScriptVmRendererDocumentIsolateOps { vm: self }
    }

    pub(super) fn renderer_document_isolate_heap_usage(
        &mut self,
    ) -> Result<RendererRuntimeHeapUsage> {
        let moli_counters = self.moli_memory_counters();
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let stats = isolate.get_heap_statistics();
                let heap_spaces = (0..isolate.number_of_heap_spaces())
                    .filter_map(|index| {
                        isolate.get_heap_space_statistics(index).map(|space| {
                            RendererRuntimeHeapSpaceUsage {
                                name: space.space_name().to_string_lossy().into_owned(),
                                size: space.space_size(),
                                used_size: space.space_used_size(),
                                available_size: space.space_available_size(),
                                physical_size: space.physical_space_size(),
                            }
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(RendererRuntimeHeapUsage {
                    used_size: stats.used_heap_size(),
                    total_size: stats.total_heap_size(),
                    total_heap_size_executable: stats.total_heap_size_executable(),
                    total_physical_size: stats.total_physical_size(),
                    total_available_size: stats.total_available_size(),
                    heap_size_limit: stats.heap_size_limit(),
                    malloced_memory: stats.malloced_memory(),
                    peak_malloced_memory: stats.peak_malloced_memory(),
                    external_memory: stats.external_memory(),
                    number_of_native_contexts: stats.number_of_native_contexts(),
                    number_of_detached_contexts: stats.number_of_detached_contexts(),
                    total_allocated_bytes: stats.total_allocated_bytes(),
                    total_global_handles_size: stats.total_global_handles_size(),
                    used_global_handles_size: stats.used_global_handles_size(),
                    heap_spaces,
                    moli: moli_counters,
                })
            })
    }

    pub(super) fn moli_memory_counters(&self) -> RendererMoliMemoryDiagnostics {
        let document = self.document_runtime.snapshot_document();
        let dom = moli_dom_memory_counters(&document);
        let main_window_proxy_identity_hash = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_runtime.context);
                Ok(context.global(scope).get_identity_hash().get())
            })
            .ok();
        let host = self._context_host.borrow();
        RendererMoliMemoryDiagnostics {
            scope: RendererMoliMemoryScopeDiagnostics {
                v8_heap: "page-vm-document-isolate",
                v8_heap_is_target_local: true,
                counters: "target-document",
                garbage_collection: "page-vm-document-isolate",
            },
            dom,
            runtime: RendererMoliRuntimeMemoryDiagnostics {
                runtime_observable_context_count: self.page_runtime_observable_contexts().len(),
                isolated_context_count: self.page_isolated_world_contexts.len(),
                child_default_context_count: self.child_frame_realm_store.len(),
                child_browsing_context_count: host.child_browsing_context_count(),
                pending_subresource_requests: host.pending_subresource_request_count(),
                pending_subresource_fetch_infos: host.pending_subresource_fetch_info_count(),
                pending_subresource_continue_events: host
                    .pending_subresource_continue_event_count(),
                pending_runtime_binding_calls: self
                    .document_runtime
                    .pending_runtime_binding_call_count(),
                completed_child_frame_navigation_loads: host
                    .completed_child_frame_navigation_load_count(),
                pending_inspector_messages: self.page_inspector.outbound_len(),
                inspector_session_registry_owner: self
                    .page_inspector
                    .registry_owner_for_diagnostics(),
                inspector_session_registry_lifetime_scope: self
                    .page_inspector
                    .registry_lifetime_scope_for_diagnostics(),
                inspector_session_count: self.page_inspector.session_count_for_diagnostics(),
                inspector_context_group_id: self.page_inspector.context_group_id_for_diagnostics(),
                inspector_context_group_scope: self
                    .page_inspector
                    .registry_lifetime_scope_for_diagnostics(),
                inspector_context_registration_count: self
                    .page_inspector
                    .context_registration_count_for_diagnostics(),
                main_window_proxy_identity_hash,
                inspector_default_context_registry_count: self
                    .renderer_document_isolate
                    .renderer_document_isolate_inspector_default_context_registry_count(),
                inspector_default_context_registry_scope: "page-vm-document-isolate",
                v8_foreground_task_wake_scope: "page-vm-document-isolate",
                v8_foreground_task_wake_context_group_id_available: false,
                v8_foreground_task_wake_internal_policy: "typed-page-source-and-owner-scheduler",
                v8_foreground_task_wake_external_policy: "post-turn-runtime-output",
            },
            script_execution: self.script_execution_memory.to_diagnostics(),
        }
    }

    pub(super) fn collect_renderer_document_isolate_garbage(&mut self) -> Result<()> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.memory_pressure_notification(v8::MemoryPressureLevel::Critical);
                isolate.low_memory_notification();
                Ok(())
            })
    }

    pub(super) fn notify_renderer_document_isolate_memory_pressure(&mut self) -> Result<()> {
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let stats = isolate.get_heap_statistics();
                if renderer_document_isolate_critical_pressure_required(
                    stats.used_heap_size(),
                    stats.heap_size_limit(),
                ) {
                    // One renderer process can host several Page isolates. A
                    // target with a rapidly replaced child realm must not hit
                    // V8's process-fatal heap limit before its owner can close
                    // that target. Escalate the existing periodic maintenance
                    // once this isolate consumes a third of its own limit.
                    isolate.memory_pressure_notification(v8::MemoryPressureLevel::Critical);
                    isolate.low_memory_notification();
                } else {
                    isolate.memory_pressure_notification(v8::MemoryPressureLevel::Moderate);
                }
                Ok(())
            })
    }

    pub(super) fn snapshot_console_messages_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
    ) -> Result<Vec<String>> {
        // SAFETY: callers pass pointers to `self.page_default_runtime.context` or page realm context entries owned by this `ScriptVm`.
        // The snapshot operation only reads a context slot; it does not mutate or remove any
        // context while the raw pointer is used.
        // This is an internal console snapshot, not an owner-visible script turn.
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            Ok(crate::context_bootstrap::snapshot_console_messages_for_current_context(scope))
        })
        .context("failed to snapshot console output")
    }

    pub(super) fn snapshot_console_message_details_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
    ) -> Result<Vec<Value>> {
        // SAFETY: callers pass pointers to `self.page_default_runtime.context` or page realm context entries owned by this `ScriptVm`.
        // The snapshot operation only reads a context slot; it does not mutate or remove any
        // context while the raw pointer is used.
        // This is an internal console snapshot, not an owner-visible script turn.
        let mut details = self
            .with_context_scope_by_ptr(context_ptr, |scope, _| {
                Ok(
                    crate::context_bootstrap::snapshot_console_message_details_for_current_context(
                        scope,
                    ),
                )
            })
            .context("failed to snapshot console detail output")?;
        if !details.is_empty() {
            return Ok(details);
        }

        details = self
            .snapshot_console_messages_in_context(context_ptr)?
            .into_iter()
            .map(|message| {
                let text = message
                    .split_once(": ")
                    .map(|(_, text)| text)
                    .unwrap_or(message.as_str())
                    .to_owned();
                json!({
                    "message": message,
                    "text": text,
                    "args": [
                        {
                            "type": "string",
                            "value": text,
                        }
                    ],
                })
            })
            .collect();
        Ok(details)
    }
}

impl ScriptVmRendererDocumentIsolateOps<'_> {
    pub(crate) fn renderer_document_isolate_heap_usage(
        &mut self,
    ) -> Result<RendererRuntimeHeapUsage> {
        self.vm.renderer_document_isolate_heap_usage()
    }

    pub(crate) fn collect_renderer_document_isolate_garbage(&mut self) -> Result<()> {
        self.vm.collect_renderer_document_isolate_garbage()
    }

    pub(crate) fn notify_renderer_document_isolate_memory_pressure(&mut self) -> Result<()> {
        self.vm.notify_renderer_document_isolate_memory_pressure()
    }
}

impl ScriptVm {
    pub(crate) fn take_network_output(&mut self) -> crate::types::ScriptNetworkOutput {
        self._context_host.borrow_mut().take_network_output()
    }

    pub(crate) fn subresource_activity_epoch(&self) -> u64 {
        self._context_host.borrow().subresource_activity_epoch()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_subresource_fetch_infos(
        &mut self,
    ) -> Vec<PendingSubresourceFetchInfo> {
        self._context_host
            .borrow_mut()
            .take_pending_subresource_fetch_infos()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_subresource_continue_events(
        &mut self,
    ) -> Vec<PendingSubresourceContinueEvent> {
        self._context_host
            .borrow_mut()
            .take_pending_subresource_continue_events()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_file_chooser_activations(
        &mut self,
    ) -> Vec<crate::RendererPendingFileChooserActivation> {
        self._context_host
            .borrow_mut()
            .take_pending_file_chooser_activations()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_download_activations(
        &mut self,
    ) -> Vec<crate::RendererPendingDownloadActivation> {
        self._context_host
            .borrow_mut()
            .take_pending_download_activations()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_javascript_dialogs(
        &mut self,
    ) -> Vec<crate::RendererPendingJavaScriptDialog> {
        self._context_host
            .borrow_mut()
            .take_pending_javascript_dialogs()
    }

    pub(crate) fn set_javascript_dialog_handler_enabled(&mut self, enabled: bool) {
        self._context_host
            .borrow_mut()
            .set_javascript_dialog_handler_enabled(enabled);
    }

    #[cfg(test)]
    pub(crate) fn take_pending_popup_activations(
        &mut self,
    ) -> Vec<crate::RendererPendingPopupActivation> {
        self._context_host
            .borrow_mut()
            .take_pending_popup_activations()
    }

    #[cfg(test)]
    pub(crate) fn take_completed_child_frame_navigation_loads(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameNavigationSnapshot> {
        self._context_host
            .borrow_mut()
            .take_completed_child_frame_navigation_loads()
            .into_iter()
            .map(
                |snapshot| crate::protocol_types::ChildFrameNavigationSnapshot {
                    frame_id: snapshot.frame_id,
                    parent_frame_id: snapshot.parent_frame_id,
                    loader_id: snapshot.loader_id,
                    name: snapshot.name,
                    url: snapshot.url,
                    document_open_replacement: snapshot.document_open_replacement,
                    security_origin_inherited: snapshot.security_origin_inherited,
                    security_origin_opaque: snapshot.security_origin_opaque,
                    document_network: snapshot.document_network,
                },
            )
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn take_completed_child_document_networks(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameDocumentNetworkActivitySnapshot> {
        self._context_host
            .borrow_mut()
            .take_completed_child_document_networks()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_child_frame_tree_events(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameTreeEventSnapshot> {
        self._context_host
            .borrow_mut()
            .take_pending_child_frame_tree_events()
    }

    #[cfg(test)]
    pub(crate) fn completed_child_frame_navigation_load_count(&self) -> usize {
        self._context_host
            .borrow()
            .completed_child_frame_navigation_load_count()
    }

    #[cfg(test)]
    pub(crate) fn take_runtime_binding_calls(&mut self) -> Vec<PendingRuntimeBindingCall> {
        drain_internal_runtime_binding_calls(self);
        self.document_runtime.take_runtime_binding_calls()
    }

    pub(crate) fn take_runtime_inspector_messages(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Vec<RendererRuntimeInspectorMessage> {
        let messages = self
            .page_inspector
            .take_outbound_messages_for_session(inspector_session_id);
        self.page_inspector
            .record_execution_context_state(&messages, self.root_frame_id.as_deref());
        self.page_isolated_world_contexts
            .record_inspector_context_state(&messages, self.root_frame_id.as_deref());
        self.runtime_inspector_messages_from_v8_messages(messages)
    }

    pub(crate) fn devtools_agent_token(&self) -> moli_page_types::RendererDevToolsAgentToken {
        self.page_inspector.agent_token()
    }

    pub(crate) fn inspector_v8_session_state(
        &self,
        inspector_session_id: Option<&str>,
    ) -> Option<moli_page_types::V8InspectorSessionState> {
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|_, _| {
                self.page_inspector.v8_session_state(inspector_session_id)
            })
    }

    pub(crate) fn inspector_v8_session_states(
        &self,
    ) -> Vec<(
        moli_page_types::DevToolsSessionKey,
        moli_page_types::V8InspectorSessionState,
    )> {
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|_, _| {
                self.page_inspector.v8_session_states()
            })
    }

    pub(crate) fn reattach_v8_inspector_sessions(
        &self,
        restores: &[crate::runtime::RendererInspectorSessionRestoreSnapshot],
    ) {
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(|_, backend| {
                self.page_inspector.reattach_v8_sessions(backend, restores);
            });
    }

    pub(crate) fn ensure_runtime_inspector_session(&mut self, inspector_session_id: Option<&str>) {
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let page_inspector = &self.page_inspector;
        renderer_document_isolate.with_renderer_document_isolate_and_inspector_mut(|_, backend| {
            page_inspector.ensure_frontend_session(backend, inspector_session_id);
        });
    }

    pub(crate) fn page_diagnostics_snapshot(&mut self) -> Result<RendererPageDiagnosticsSnapshot> {
        self.sync_runtime_observable_source_events()?;
        let runtime_observable_source = self
            .runtime_observable_source_queue
            .snapshot(self.runtime_observable_default_execution_context_id());
        let runtime_console_messages_by_context = runtime_observable_source
            .as_ref()
            .map(|source| source.console_messages_by_context())
            .unwrap_or_default();
        let runtime_console_messages_with_context = runtime_observable_source
            .as_ref()
            .map(|source| source.console_messages_with_context())
            .unwrap_or_default();
        let runtime_lifecycle_errors = runtime_observable_source
            .as_ref()
            .map(|source| source.lifecycle_errors())
            .unwrap_or_default();
        let host = self._context_host.borrow();
        let mut snapshot =
            RendererPageDiagnosticsSnapshot::from_diagnostics(RendererActivityDiagnostics {
                document_context_count: 1
                    + self.page_isolated_world_contexts.len()
                    + self.child_frame_realm_store.len(),
                isolated_world_context_count: self.page_isolated_world_contexts.len(),
                child_default_context_count: self.child_frame_realm_store.len(),
                pending_subresource_requests: host.pending_subresource_request_count(),
                pending_subresource_fetch_infos: host.pending_subresource_fetch_info_count(),
                pending_subresource_continue_events: host
                    .pending_subresource_continue_event_count(),
                pending_file_chooser_activations: host.pending_file_chooser_activation_count(),
                pending_download_activations: host.pending_download_activation_count(),
                pending_popup_activations: host.pending_popup_activation_count(),
                pending_javascript_dialogs: host.pending_javascript_dialog_count(),
                pending_runtime_binding_calls: self
                    .document_runtime
                    .pending_runtime_binding_call_count(),
                pending_inspector_messages: self.page_inspector.outbound_len(),
                runtime_console_messages_with_context,
                runtime_console_messages_by_context,
                runtime_lifecycle_errors,
                completed_child_frame_navigation_loads: host
                    .completed_child_frame_navigation_load_count(),
                dedicated_worker_loading_count: host
                    .dedicated_worker_loading_count_for_diagnostics(),
                dedicated_worker_running_worker_isolate_count: host
                    .dedicated_worker_running_worker_isolate_count_for_diagnostics(),
                pending_webcrypto_tasks: host.pending_webcrypto_task_count(),
                pending_opfs_tasks: host.pending_opfs_task_count(),
            });
        snapshot.set_runtime_observable_source(runtime_observable_source);
        Ok(snapshot)
    }

    pub(crate) fn dedicated_worker_running_worker_isolate_count_for_diagnostics(&self) -> usize {
        self._context_host
            .borrow()
            .dedicated_worker_running_worker_isolate_count_for_diagnostics()
    }

    pub(crate) fn has_pending_bitmap_tasks(&self) -> bool {
        self._context_host.borrow().has_pending_bitmap_tasks()
    }
    pub(crate) fn has_pending_webcrypto_tasks(&self) -> bool {
        self._context_host.borrow().has_pending_webcrypto_tasks()
    }

    #[cfg(test)]
    pub(crate) fn pending_webcrypto_execution_contexts_for_test(
        &self,
    ) -> Vec<(
        crate::native_bridge::WindowExecutionContextOwner,
        crate::native_bridge::RuntimeObservableContextToken,
    )> {
        self._context_host
            .borrow()
            .pending_webcrypto_execution_contexts_for_test()
    }

    pub(crate) fn has_pending_opfs_tasks(&self) -> bool {
        self._context_host.borrow().has_pending_opfs_tasks()
    }

    pub(crate) fn pending_subresource_request_count(&self) -> usize {
        self._context_host
            .borrow()
            .pending_subresource_request_count()
    }

    pub(crate) fn has_pending_native_module_job(&self) -> bool {
        self.has_pending_dynamic_module_job()
    }

    pub(crate) fn has_pending_dynamic_module_job(&self) -> bool {
        self.document_runtime
            .has_pending_native_dynamic_module_import()
            || self.has_pending_child_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_ready_dynamic_module_job(&self) -> bool {
        self.document_runtime
            .has_ready_native_dynamic_module_import()
    }

    #[cfg(test)]
    pub(crate) fn has_inflight_dynamic_module_fetch(&self) -> bool {
        self.document_runtime
            .has_inflight_native_dynamic_module_import_fetch()
            || self.has_inflight_child_dynamic_module_import_fetch()
    }

    pub(crate) fn resource_scheduler(&self) -> RendererResourceScheduler {
        self._context_host.borrow().resource_scheduler()
    }

    pub(crate) fn accept_parser_discovered_native_modulepreloads(
        &mut self,
        link_handles: impl IntoIterator<Item = crate::dom::native::NativeNodeId>,
    ) -> bool {
        let (preloads, runtime_warnings, link_error_tasks) = self
            .document_runtime
            .accept_parser_discovered_modulepreload_links(link_handles)
            .into_parts();
        let mut progressed = link_error_tasks > 0;
        for warning in runtime_warnings {
            self.record_runtime_warning(format_args!("{warning}"));
            progressed = true;
        }
        for preload in preloads {
            match self.register_native_modulepreload_for_owner(preload) {
                Ok(run) => progressed |= run.is_some(),
                Err(error) => self.record_runtime_warning(format_args!(
                    "parser-discovered modulepreload failed before fetch scheduling: {}",
                    error
                )),
            }
        }
        progressed
    }
}
