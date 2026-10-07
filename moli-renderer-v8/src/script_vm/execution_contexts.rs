use super::*;

impl ScriptVm {
    pub(super) fn child_frame_realm_context_ptr(
        &self,
        realm_id: FrameRealmId,
    ) -> Result<*const v8::Global<v8::Context>> {
        self.child_frame_realm_store
            .context_for_owner_realm_id(realm_id)
            .map(|realm| &realm.context as *const _)
            .ok_or_else(|| anyhow!("unknown child frame owner realm `{realm_id:?}`"))
    }

    pub(super) fn child_frame_realm_context_ptr_for_execution_context_id(
        &self,
        execution_context_id: i64,
    ) -> Result<*const v8::Global<v8::Context>> {
        let realm_id = self
            .child_frame_realm_store
            .owner_realm_id_for_context_id(execution_context_id)
            .ok_or_else(|| anyhow!("unknown child frame realm `{execution_context_id}`"))?;
        self.child_frame_realm_context_ptr(realm_id)
    }

    pub(super) fn inspector_window_dispatch_scope_for_target(
        &self,
        target: InspectorWindowDispatchTarget,
    ) -> Option<InspectorWindowDispatchScope> {
        let execution_context_id = match target {
            InspectorWindowDispatchTarget::DefaultTop => {
                return Some(InspectorWindowDispatchScope {
                    context_ptr: &self.page_default_context as *const _,
                    child_handle: None,
                });
            }
            InspectorWindowDispatchTarget::ExecutionContext(execution_context_id) => {
                execution_context_id
            }
        };
        if self.runtime_observable_default_execution_context_id() == Some(execution_context_id) {
            return Some(InspectorWindowDispatchScope {
                context_ptr: &self.page_default_context as *const _,
                child_handle: None,
            });
        }
        if let Some(realm) = self.child_frame_realm_store.get(&execution_context_id) {
            return Some(InspectorWindowDispatchScope {
                context_ptr: &realm.context as *const _,
                child_handle: Some(realm.child_handle),
            });
        }
        let isolated_context_id = self
            .page_isolated_world_contexts
            .execution_context_id_for_inspector_context(execution_context_id)?;
        let world = self
            .page_isolated_world_contexts
            .context(isolated_context_id)?;
        Some(InspectorWindowDispatchScope {
            context_ptr: &world.context as *const _,
            child_handle: world.child_handle,
        })
    }

    pub(super) fn frame_realm_context_ptr(
        &self,
        realm_id: FrameRealmId,
    ) -> Result<*const v8::Global<v8::Context>> {
        if realm_id.0 == 0 {
            return Ok(&self.page_default_context as *const _);
        }
        self.child_frame_realm_context_ptr(realm_id)
    }

    pub(super) fn child_frame_owner_realm_id_for_execution_context_id(
        &self,
        execution_context_id: i64,
    ) -> Result<FrameRealmId> {
        let context = self
            .child_frame_realm_store
            .get(&execution_context_id)
            .ok_or_else(|| anyhow!("unknown child frame realm `{execution_context_id}`"))?;
        let host = self._context_host.borrow();
        let owner_realm_id = context.owner_realm_id;
        let owner = host
            .frame_owner_current_child_snapshot_for_realm(owner_realm_id)
            .ok_or_else(|| {
                anyhow!(
                    "child frame realm `{execution_context_id}` has no current FrameOwnerStore snapshot"
                )
            })?;
        if owner.owner_handle != context.child_handle
            || owner.frame_id.0.as_str() != context.frame_id
        {
            return Err(anyhow!(
                "child frame realm `{execution_context_id}` maps to stale owner realm {owner_realm_id:?}"
            ));
        }
        Ok(owner_realm_id)
    }

    pub(super) fn with_child_frame_realm_context_scope<T>(
        &mut self,
        execution_context_id: i64,
        op: impl FnOnce(&mut v8::PinScope<'_, '_>, *mut JsContextHost) -> Result<T>,
    ) -> Result<T> {
        let realm_id =
            self.child_frame_owner_realm_id_for_execution_context_id(execution_context_id)?;
        self.with_frame_realm_scope(realm_id, op)
    }

    pub(super) fn child_frame_source_script_job_for_execution_context_id(
        &self,
        execution_context_id: i64,
        kind: FrameScriptJobKind,
        source: String,
    ) -> Result<FrameScriptJob> {
        let context = self
            .child_frame_realm_store
            .get(&execution_context_id)
            .ok_or_else(|| anyhow!("unknown child frame realm `{execution_context_id}`"))?;
        self._context_host
            .borrow()
            .frame_owner_child_source_script_job(context.child_handle, kind, source)
            .ok_or_else(|| {
                anyhow!(
                    "child frame realm `{execution_context_id}` has no current FrameScriptJob owner"
                )
            })
    }

    pub(super) fn exec_child_frame_source_script_job_for_execution_context_id(
        &mut self,
        execution_context_id: i64,
        kind: FrameScriptJobKind,
        source: &str,
    ) -> Result<()> {
        let job = self.child_frame_source_script_job_for_execution_context_id(
            execution_context_id,
            kind,
            source.to_owned(),
        )?;
        self.exec_frame_script_job(job)
    }

    pub(crate) fn default_execution_context_id(&self) -> Option<i64> {
        self.page_inspector.default_execution_context_id()
    }

    pub(crate) fn default_or_initial_execution_context_id(&self) -> Option<i64> {
        self.page_inspector
            .default_execution_context_id()
            .or_else(|| self.page_inspector.initial_default_execution_context_id())
    }

    pub(super) fn runtime_observable_default_execution_context_id(&self) -> Option<i64> {
        // Creation-time console output can be observed before Runtime.enable has
        // materialized the session-visible default context id. The renderer
        // still owns the initial V8 context identity, so source items can be
        // emitted immediately instead of being parked as contextless messages.
        self.page_inspector
            .default_execution_context_id()
            .or_else(|| self.page_inspector.initial_default_execution_context_id())
    }

    pub(super) fn runtime_observable_default_execution_context_realm_id(&self) -> Option<String> {
        self.page_inspector
            .default_execution_context_realm_id()
            .or_else(|| {
                self.page_inspector
                    .initial_default_execution_context_realm_id()
            })
    }

    pub(crate) fn root_frame_id(&self) -> Option<&str> {
        self.root_frame_id.as_deref()
    }

    pub(super) fn default_runtime_realm_info(&self) -> Option<RendererRuntimeRealmInfo> {
        let context_id = self.runtime_observable_default_execution_context_id()?;
        let document_url = self.document_runtime.document_url();
        Some(RendererRuntimeRealmInfo {
            context_id,
            realm_id: self.runtime_observable_default_execution_context_realm_id(),
            frame_id: self.root_frame_id.clone(),
            origin: moli_url::origin_ascii_serialization(document_url),
            name: document_url.as_str().to_owned(),
            is_default: true,
            context_type: "default".to_owned(),
            grant_universal_access: None,
        })
    }

    pub(crate) fn runtime_realm_inventory(&mut self) -> Vec<RendererRuntimeRealmInfo> {
        self.prune_stale_child_default_execution_contexts();
        self.known_runtime_realm_inventory()
    }

    pub(crate) fn known_runtime_realm_inventory(&self) -> Vec<RendererRuntimeRealmInfo> {
        let mut realms = Vec::new();
        realms.extend(self.default_runtime_realm_info());

        let mut isolated_context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids()
            .collect::<Vec<_>>();
        isolated_context_ids.sort_unstable();
        realms.extend(
            isolated_context_ids
                .into_iter()
                .filter_map(|context_id| self.isolated_world_runtime_realm_info(context_id)),
        );

        let mut child_context_ids = self
            .child_frame_realm_store
            .execution_context_ids()
            .collect::<Vec<_>>();
        child_context_ids.sort_unstable();
        realms.extend(
            child_context_ids
                .into_iter()
                .filter_map(|context_id| self.child_default_runtime_realm_info(context_id)),
        );

        realms
    }

    pub(super) fn runtime_inspector_messages_from_v8_messages(
        &self,
        messages: impl IntoIterator<Item = Value>,
    ) -> Vec<RendererRuntimeInspectorMessage> {
        messages
            .into_iter()
            .map(RendererRuntimeInspectorMessage::from_v8_inspector_message)
            .collect()
    }

    pub(crate) fn has_isolated_execution_context_id(&self, execution_context_id: i64) -> bool {
        self.page_isolated_world_contexts
            .has_execution_context_id(execution_context_id)
    }

    pub(crate) fn has_isolated_world_named(&self, name: &str) -> bool {
        self.page_isolated_world_contexts
            .has_world_for_scope(None, name)
    }

    pub(crate) fn has_isolated_world_named_for_frame(&self, frame_id: &str, name: &str) -> bool {
        self.page_isolated_world_contexts
            .has_world_for_scope(Some(frame_id), name)
    }

    pub(crate) fn has_isolated_world_for_owner(
        &self,
        devtools_session: Option<&DevToolsSessionKey>,
        frame_id: Option<&str>,
        name: &str,
    ) -> bool {
        self.page_isolated_world_contexts
            .execution_context_id_for_scope(devtools_session, frame_id, name)
            .is_some()
    }

    pub(crate) fn inspector_execution_context_id_for_isolated_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        self.page_isolated_world_contexts
            .inspector_execution_context_id(execution_context_id)
    }

    pub(crate) fn isolated_execution_context_id_for_inspector_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        self.page_isolated_world_contexts
            .execution_context_id_for_inspector_context(execution_context_id)
    }

    pub(crate) fn child_default_frame_id_for_execution_context_id(
        &mut self,
        execution_context_id: i64,
    ) -> Option<String> {
        self.prune_stale_child_default_execution_contexts();
        let context = self.child_frame_realm_store.get(&execution_context_id)?;
        self._context_host
            .borrow()
            .frame_owner_frame_id_for_child_handle(context.child_handle)
            .map(|frame_id| frame_id.0)
            .or_else(|| Some(context.frame_id.clone()))
    }

    pub(crate) fn child_default_execution_context_id_for_frame_id(
        &mut self,
        frame_id: &str,
    ) -> Option<i64> {
        self.prune_stale_child_default_execution_contexts();
        self.child_frame_realm_store
            .iter_by_execution_context_id()
            .find_map(|(execution_context_id, context)| {
                (context.frame_id == frame_id).then_some(execution_context_id)
            })
    }

    pub(crate) fn child_browsing_context_module_request_initiator_url(
        &self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Option<Url> {
        self._context_host
            .borrow()
            .child_browsing_context_request_initiator_url(child_handle)
    }

    pub(super) fn isolated_world_runtime_realm_info(
        &self,
        context_id: i64,
    ) -> Option<RendererRuntimeRealmInfo> {
        let world = self.page_isolated_world_contexts.context(context_id)?;
        let origin = world
            .child_handle
            .and_then(|handle| {
                self._context_host
                    .borrow()
                    .child_browsing_context_current_url(handle)
            })
            .unwrap_or_else(|| self.document_runtime.document_url().clone());
        Some(RendererRuntimeRealmInfo {
            context_id,
            realm_id: world.inspector_execution_context_realm_id.clone(),
            frame_id: world
                .frame_id
                .clone()
                .or_else(|| self.root_frame_id.clone()),
            origin: moli_url::origin_ascii_serialization(&origin),
            name: world.name.clone(),
            is_default: false,
            context_type: "isolated".to_owned(),
            grant_universal_access: Some(world.grant_universal_access),
        })
    }

    pub(super) fn child_default_runtime_realm_info(
        &self,
        context_id: i64,
    ) -> Option<RendererRuntimeRealmInfo> {
        let context = self.child_frame_realm_store.get(&context_id)?;
        let document_url = self
            ._context_host
            .borrow()
            .child_browsing_context_current_url(context.child_handle)
            .unwrap_or_else(|| self.document_runtime.document_url().clone());
        Some(RendererRuntimeRealmInfo {
            context_id,
            realm_id: context.inspector_execution_context_realm_id.clone(),
            frame_id: Some(context.frame_id.clone()),
            origin: moli_url::origin_ascii_serialization(&document_url),
            name: document_url.as_str().to_owned(),
            is_default: true,
            context_type: "default".to_owned(),
            grant_universal_access: None,
        })
    }

    pub(crate) fn live_child_default_runtime_realm_inventory(
        &mut self,
    ) -> Vec<RendererRuntimeRealmInfo> {
        self.prune_stale_child_default_execution_contexts();
        let mut child_context_ids = self
            .child_frame_realm_store
            .execution_context_ids()
            .collect::<Vec<_>>();
        child_context_ids.sort_unstable();
        child_context_ids
            .into_iter()
            .filter_map(|context_id| self.child_default_runtime_realm_info(context_id))
            .collect()
    }

    pub(super) fn live_child_default_context_entries(&self) -> Vec<LiveChildDefaultContextEntry> {
        let host = self._context_host.borrow();
        host.live_child_browsing_context_owner_snapshots()
            .into_iter()
            .map(|(handle, owner)| LiveChildDefaultContextEntry {
                handle,
                frame_id: owner.frame_id.0,
                owner_realm_id: owner.realm_id,
            })
            .collect()
    }

    pub(super) fn create_new_child_default_world(
        &mut self,
        frame_id: &str,
        child_handle: DomHandle,
    ) -> Result<ChildFrameRealmRecord> {
        let current_owner = self
            ._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
            .ok_or_else(|| anyhow!("child frame `{frame_id}` has no current LocalWindow owner"))?;
        let prebootstrapped = self
            .prebootstrapped_child_default_contexts
            .borrow_mut()
            .remove(&child_handle);
        let prebootstrapped = match prebootstrapped {
            Some(context) if context.local_window_id == current_owner.local_window_id => {
                Some(context)
            }
            Some(context) => {
                let context_ptr = &context.context as *const v8::Global<v8::Context>;
                self.clear_context_wrapper_cache_for_context_ptr(context_ptr, false);
                self.cancel_history_traversals_for_retiring_window(
                    crate::native_bridge::WindowExecutionContextOwner::Frame(
                        context.local_window_id,
                    ),
                );
                {
                    let mut host = self._context_host.borrow_mut();
                    host.retire_window_realm_resources(context.runtime_observable_context_token);
                    host.retire_window_execution_contexts_for_context_token(
                        context.runtime_observable_context_token,
                    );
                }
                let context_ptr = &context.context as *const v8::Global<v8::Context>;
                let reuses_window_proxy = self
                    ._context_host
                    .borrow()
                    .child_window_proxy_frame_is_current(child_handle, &context.frame_id);
                self.renderer_document_isolate
                    .with_entered_renderer_document_isolate(|isolate| {
                        let scope = pin!(v8::HandleScope::new(isolate));
                        let scope = &mut scope.init();
                        let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                        if reuses_window_proxy {
                            context.detach_global();
                        }
                        Ok(())
                    })?;
                // Cancellation can synchronously install a successor realm or
                // detach the frame. Resolve both owner and pending context anew.
                return self.create_new_child_default_world(frame_id, child_handle);
            }
            None => None,
        };
        let context_host = self._context_host.clone();
        let (context, bridge_ref, runtime_observable_context_token) =
            if let Some(prebootstrapped) = prebootstrapped {
                (
                    prebootstrapped.context,
                    prebootstrapped.bridge_ref,
                    prebootstrapped.runtime_observable_context_token,
                )
            } else {
                let context_bootstrap = self
                    .renderer_document_isolate
                    .with_entered_renderer_document_isolate_and_bootstrap(
                        |isolate, isolate_bootstrap| {
                            ScriptVmContextBootstrap::new_child_default(
                                isolate,
                                isolate_bootstrap,
                                context_host,
                                self.resource_owner_id,
                                &self.promise_reject_dispatch,
                                self.indexed_db_manager.clone(),
                                Some(self.storage_bucket_store.clone()),
                                child_handle,
                                current_owner,
                            )
                        },
                    )?;
                let runtime_observable_context_token =
                    context_bootstrap.runtime_observable_context_token;
                let (context, bridge_ref) = context_bootstrap.into_context_and_bridge_ref();
                (context, bridge_ref, runtime_observable_context_token)
            };
        let document_url = self
            ._context_host
            .borrow()
            .child_browsing_context_current_url(child_handle)
            .unwrap_or_else(|| self.document_runtime.document_url().clone());
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let inspector_document_isolate = renderer_document_isolate.clone();
        let page_inspector = &mut self.page_inspector;
        let (inspector_context, inspector_context_registration_id) = renderer_document_isolate
            .with_entered_renderer_document_isolate_and_inspector_mut(|isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let local_context = v8::Local::new(scope, &context);
                let registered_context = v8::Global::new(scope.as_ref(), local_context);
                page_inspector.attach_child_default_context(
                    inspector_document_isolate,
                    inspector,
                    local_context,
                    registered_context,
                    &document_url,
                    frame_id,
                )
            })?;
        let owner_realm_id = self
            ._context_host
            .borrow_mut()
            .bind_child_default_execution_context_id(
                child_handle,
                current_owner,
                inspector_context.id,
            );
        let Some(owner_realm_id) = owner_realm_id else {
            assert!(
                self.page_inspector
                    .destroy_context_registration(inspector_context_registration_id),
                "failed child realm materialization must release its Inspector registration"
            );
            return Err(anyhow!(
                "child frame `{frame_id}` has no current FrameOwnerStore record for FrameRealm materialization"
            ));
        };
        Ok(ChildFrameRealmRecord {
            frame_id: frame_id.to_owned(),
            child_handle,
            local_window_id: current_owner.local_window_id,
            owner_realm_id,
            context,
            _bridge_ref: bridge_ref,
            runtime_observable_context_token,
            inspector_execution_context_id: inspector_context.id,
            inspector_execution_context_realm_id: inspector_context.unique_id,
            inspector_context_registration_id,
        })
    }

    pub(super) fn prune_stale_child_default_execution_contexts(&mut self) {
        let live = self.live_child_default_context_entries();
        self.prune_stale_child_default_execution_contexts_for_live_entries(&live);
    }

    pub(super) fn prune_stale_child_default_execution_contexts_for_live_entries(
        &mut self,
        live: &[LiveChildDefaultContextEntry],
    ) {
        let stale_prebootstrapped_handles = {
            let host = self._context_host.borrow();
            self.prebootstrapped_child_default_contexts
                .borrow()
                .iter()
                .filter_map(|(handle, context)| {
                    (host
                        .current_child_document_task_owner(*handle)
                        .map(|owner| owner.local_window_id)
                        != Some(context.local_window_id))
                    .then_some(*handle)
                })
                .collect::<Vec<_>>()
        };
        let stale_prebootstrapped_contexts = {
            let mut contexts = self.prebootstrapped_child_default_contexts.borrow_mut();
            stale_prebootstrapped_handles
                .into_iter()
                .filter_map(|handle| contexts.remove(&handle).map(|context| (handle, context)))
                .collect::<Vec<_>>()
        };
        if !stale_prebootstrapped_contexts.is_empty() {
            for (_, context) in &stale_prebootstrapped_contexts {
                let context_ptr = &context.context as *const v8::Global<v8::Context>;
                self.clear_context_wrapper_cache_for_context_ptr(context_ptr, false);
                self.cancel_history_traversals_for_retiring_window(
                    crate::native_bridge::WindowExecutionContextOwner::Frame(
                        context.local_window_id,
                    ),
                );
            }
            {
                let mut host = self._context_host.borrow_mut();
                for (_, context) in &stale_prebootstrapped_contexts {
                    host.retire_window_realm_resources(context.runtime_observable_context_token);
                    host.retire_window_execution_contexts_for_context_token(
                        context.runtime_observable_context_token,
                    );
                }
            }
            let context_host = self._context_host.clone();
            let _ = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    for (handle, context) in &stale_prebootstrapped_contexts {
                        if context_host
                            .borrow()
                            .child_window_proxy_frame_is_current(*handle, &context.frame_id)
                        {
                            v8::Local::new(scope, &context.context).detach_global();
                        }
                    }
                    Ok(())
                });
        }
        let refreshed_live;
        let live = if stale_prebootstrapped_contexts.is_empty() {
            live
        } else {
            // Retirement callbacks may have materialized successor realms.
            refreshed_live = self.live_child_default_context_entries();
            &refreshed_live
        };
        let stale_context_ids = self
            .child_frame_realm_store
            .iter_by_execution_context_id()
            .filter_map(|(context_id, context)| {
                let live_entry = live
                    .iter()
                    .find(|entry| entry.handle == context.child_handle);
                let is_stale = live_entry
                    .map(|entry| entry.owner_realm_id != Some(context.owner_realm_id))
                    .unwrap_or(true)
                    || live_entry
                        .map(|entry| entry.frame_id != context.frame_id)
                        .unwrap_or(true)
                    || self
                        .child_frame_realm_store
                        .context_id_for_owner_realm_id(context.owner_realm_id)
                        != Some(context_id);
                is_stale.then_some(context_id)
            })
            .collect::<Vec<_>>();
        for context_id in stale_context_ids {
            self.destroy_child_default_context(context_id);
        }
    }

    pub(super) fn destroy_child_default_context(&mut self, execution_context_id: i64) {
        let Some(context) = self.child_frame_realm_store.remove(&execution_context_id) else {
            return;
        };
        let context_ptr: *const v8::Global<v8::Context> = &context.context as *const _;
        self.clear_context_wrapper_cache_for_context_ptr(context_ptr, false);
        self.cancel_history_traversals_for_retiring_window(
            crate::native_bridge::WindowExecutionContextOwner::Frame(context.local_window_id),
        );
        {
            let mut host = self._context_host.borrow_mut();
            host.clear_child_default_execution_context_if_matches(
                context.child_handle,
                context.owner_realm_id,
                execution_context_id,
            );
            host.retire_window_realm_resources(context.runtime_observable_context_token);
            host.retire_window_execution_contexts_for_context_token(
                context.runtime_observable_context_token,
            );
        }
        assert!(
            self.page_inspector
                .destroy_context_registration(context.inspector_context_registration_id),
            "child default context must retain its document-owned Inspector registration"
        );
        let context_host = self._context_host.clone();
        let proxy_cleanup_result = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let local_context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let host_ptr = (*context_host).as_ptr();
                let host = unsafe { &mut *host_ptr };
                let reuses_window_proxy = host
                    .child_window_proxy_frame_is_current(context.child_handle, &context.frame_id);
                // Navigation within the same frame reuses its WindowProxy.
                // Removal (including remove/reinsert of the same element)
                // creates a different browsing context. Keep the old proxy
                // attached to its retained Window so its constructors remain
                // callable after execution authority has been retired.
                if reuses_window_proxy {
                    local_context.detach_global();
                    if !host.preserve_child_window_proxy_between_realms(scope, context.child_handle)
                    {
                        anyhow::bail!("failed to park the live child WindowProxy between realms");
                    }
                }
                Ok(reuses_window_proxy)
            });
        match proxy_cleanup_result {
            Err(error) => tracing::warn!(
                %error,
                execution_context_id,
                child_handle = context.child_handle.index(),
                owner_realm_id = ?context.owner_realm_id,
                "failed to finalize retired child WindowProxy"
            ),
            Ok(reuses_window_proxy) => tracing::debug!(
                execution_context_id,
                child_handle = context.child_handle.index(),
                owner_realm_id = ?context.owner_realm_id,
                reuses_window_proxy,
                "finalized retired child WindowProxy"
            ),
        }
    }

    pub(super) fn destroy_isolated_world_context(&mut self, execution_context_id: i64) {
        let Some(context) = self
            .page_isolated_world_contexts
            .remove_context(execution_context_id)
        else {
            return;
        };
        self._context_host
            .borrow_mut()
            .retire_window_realm_resources(context.runtime_observable_context_token);
        assert!(
            self.page_inspector
                .destroy_context_registration(context.inspector_context_registration_id),
            "isolated context must retain its document-owned Inspector registration"
        );
        let context_ptr: *const v8::Global<v8::Context> = &context.context as *const _;
        self.clear_context_wrapper_cache_for_context_ptr(context_ptr, false);
        // V8 Inspector consumes the still-identifiable realm while processing
        // `context_destroyed` (including Runtime lifecycle projection for
        // named child worlds). Only after that notification may the strict
        // realm locator become stale. Isolated worlds have no owner-indexed
        // default-world binding, so retire only this token's registry entry.
        let retired_window_execution_context_realm_count = self
            ._context_host
            .borrow_mut()
            .retire_isolated_window_execution_context(context.runtime_observable_context_token);
        tracing::debug!(
            execution_context_id,
            context_token = ?context.runtime_observable_context_token,
            retired_window_execution_context_realm_count,
            "retired isolated-world Runtime binding context"
        );
    }

    pub(super) fn retire_isolated_worlds_for_document_owner(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> usize {
        let stale_context_ids = self
            .page_isolated_world_contexts
            .contexts_with_ids()
            .filter_map(|(context_id, world)| (world.document_owner == owner).then_some(context_id))
            .collect::<Vec<_>>();
        let retired_count = stale_context_ids.len();
        for context_id in stale_context_ids {
            self.destroy_isolated_world_context(context_id);
        }
        retired_count
    }

    pub(super) fn retire_isolated_worlds_for_devtools_session(
        &mut self,
        devtools_session: &DevToolsSessionKey,
    ) -> usize {
        let context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids_for_devtools_session(devtools_session);
        let retired_count = context_ids.len();
        for context_id in context_ids {
            self.destroy_isolated_world_context(context_id);
        }
        retired_count
    }

    pub(super) fn rebind_isolated_worlds_for_document_owner_transition(
        &mut self,
        retired_owner: FrameDocumentTaskOwner,
        current_owner: FrameDocumentTaskOwner,
    ) -> usize {
        let targets = self
            .page_isolated_world_contexts
            .contexts_with_ids()
            .filter_map(|(execution_context_id, world)| {
                (world.document_owner == retired_owner).then_some((
                    execution_context_id,
                    world.child_handle,
                    world.runtime_observable_context_token,
                ))
            })
            .collect::<Vec<_>>();
        let mut rebound_count = 0;
        for (execution_context_id, child_handle, realm_token) in targets {
            let rebound = if let Some(child_handle) = child_handle {
                let Some(context_ptr) = self
                    .page_isolated_world_contexts
                    .context(execution_context_id)
                    .map(|world| &world.context as *const v8::Global<v8::Context>)
                else {
                    continue;
                };
                let context_host = self._context_host.clone();
                let result = self
                    .renderer_document_isolate
                    .with_entered_renderer_document_isolate(|isolate| {
                        let scope = pin!(v8::HandleScope::new(isolate));
                        let scope = &mut scope.init();
                        let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                        let child_scope = &mut v8::ContextScope::new(scope, context);
                        let global = context.global(child_scope);
                        context_host
                            .borrow_mut()
                            .rebind_child_window_realm_document_state(
                                child_scope,
                                global,
                                child_handle,
                                retired_owner,
                                current_owner,
                                realm_token,
                            )
                    });
                if let Err(error) = result {
                    tracing::warn!(
                        execution_context_id,
                        ?child_handle,
                        ?retired_owner,
                        ?current_owner,
                        %error,
                        "failed closed while rebinding isolated-world document state"
                    );
                    false
                } else {
                    true
                }
            } else {
                true
            };
            if !rebound {
                continue;
            }
            if let Some(world) = self
                .page_isolated_world_contexts
                .context_mut(execution_context_id)
                && world.document_owner == retired_owner
            {
                world.document_owner = current_owner;
                rebound_count += 1;
            }
        }
        rebound_count
    }

    pub(super) fn create_new_isolated_world(
        &mut self,
        devtools_session: Option<DevToolsSessionKey>,
        name: &str,
        grant_universal_access: bool,
        frame_id: Option<String>,
        child_handle: Option<DomHandle>,
    ) -> Result<i64> {
        let document_owner = match child_handle {
            Some(child_handle) => self
                ._context_host
                .borrow()
                .current_child_document_task_owner(child_handle)
                .ok_or_else(|| {
                    anyhow!(
                        "cannot create isolated world for child without a current document owner"
                    )
                })?,
            None => self
                .current_main_document_task_owner()
                .ok_or_else(|| anyhow!("cannot create isolated world without a main document"))?,
        };
        let context_host = self._context_host.clone();
        let context_bootstrap = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate_and_bootstrap(
                |isolate, isolate_bootstrap| {
                    ScriptVmContextBootstrap::new_isolated(
                        isolate,
                        isolate_bootstrap,
                        context_host,
                        self.resource_owner_id,
                        &self.promise_reject_dispatch,
                        self.indexed_db_manager.clone(),
                        Some(self.storage_bucket_store.clone()),
                        child_handle,
                        document_owner,
                        if grant_universal_access {
                            crate::native_bridge::WindowExecutionContextAccessPolicy::Universal
                        } else {
                            crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin
                        },
                    )
                },
            )?;
        let runtime_observable_context_token = context_bootstrap.runtime_observable_context_token;
        let (context, bridge_ref) = context_bootstrap.into_context_and_bridge_ref();
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        let inspector_document_isolate = renderer_document_isolate.clone();
        let page_inspector = &mut self.page_inspector;
        let inspector_frame_id = frame_id.as_deref().or(self.root_frame_id.as_deref());
        let inspector_context = renderer_document_isolate
            .with_entered_renderer_document_isolate_and_inspector_mut(|isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let local_context = v8::Local::new(scope, &context);
                let registered_context = v8::Global::new(scope.as_ref(), local_context);
                page_inspector.attach_isolated_context(
                    inspector_document_isolate,
                    inspector,
                    local_context,
                    registered_context,
                    None,
                    name,
                    grant_universal_access,
                    inspector_frame_id,
                )
            })?;
        let (inspector_context, inspector_context_registration_id) = inspector_context
            .ok_or_else(|| anyhow!("V8 inspector did not report isolated execution context id"))?;
        let execution_context_id = inspector_context.id;
        assert!(
            !self
                .page_isolated_world_contexts
                .has_execution_context_id(execution_context_id),
            "new isolated world reused a live execution context id"
        );
        let replaced_context = self.page_isolated_world_contexts.insert_context(
            execution_context_id,
            PageIsolatedWorldContext {
                name: name.to_owned(),
                devtools_session,
                grant_universal_access,
                frame_id,
                child_handle,
                document_owner,
                context,
                _bridge_ref: bridge_ref,
                runtime_observable_context_token,
                inspector_execution_context_id: Some(inspector_context.id),
                inspector_execution_context_realm_id: inspector_context.unique_id,
                inspector_context_registration_id,
            },
        );
        debug_assert!(replaced_context.is_none());
        let isolated_dispatch_scope = child_handle
            .map(crate::native_bridge::OwnerDispatchScope::Child)
            .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
        let isolated_execution_context_owner =
            crate::native_bridge::WindowExecutionContextOwner::Frame(
                document_owner.local_window_id,
            );
        if !self
            ._context_host
            .borrow_mut()
            .register_window_execution_context_realm(
                isolated_execution_context_owner,
                isolated_dispatch_scope,
                runtime_observable_context_token,
                if grant_universal_access {
                    crate::native_bridge::WindowExecutionContextAccessPolicy::Universal
                } else {
                    crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin
                },
            )
        {
            let context = self
                .page_isolated_world_contexts
                .remove_context(execution_context_id);
            if let Some(context) = context {
                self.page_inspector
                    .destroy_context_registration(context.inspector_context_registration_id);
            }
            return Err(anyhow!(
                "failed to register isolated Window execution-context realm"
            ));
        }
        if let Some(child_handle) = child_handle {
            let context_ptr = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .map(|world| &world.context as *const v8::Global<v8::Context>)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            let context_host = self._context_host.clone();
            self.renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let child_scope = &mut v8::ContextScope::new(scope, context);
                    context_host
                        .borrow_mut()
                        .bind_child_window_indexed_db_factory_after_context_registration(
                            child_scope,
                            child_handle,
                        );
                    Ok(())
                })?;
        }
        Ok(execution_context_id)
    }

    pub(crate) fn ensure_isolated_worlds_attached_to_inspector(&mut self) -> Result<()> {
        let pending_ids = self
            .page_isolated_world_contexts
            .pending_inspector_attachment_ids();
        for execution_context_id in pending_ids {
            let context_ptr: *const v8::Global<v8::Context> = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .map(|world| &world.context as *const _)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            let (name, grant_universal_access, frame_id, replaced_registration_id) = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .map(|world| {
                    (
                        world.name.clone(),
                        world.grant_universal_access,
                        world.frame_id.clone(),
                        world.inspector_context_registration_id,
                    )
                })
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            let inspector_frame_id = frame_id.as_deref().or(self.root_frame_id.as_deref());
            let renderer_document_isolate = self.renderer_document_isolate.clone();
            let inspector_document_isolate = renderer_document_isolate.clone();
            let page_inspector = &mut self.page_inspector;
            let inspector_context = renderer_document_isolate
                .with_entered_renderer_document_isolate_and_inspector_mut(
                    |isolate, inspector| {
                        let scope = pin!(v8::HandleScope::new(isolate));
                        let scope = &mut scope.init();
                        let local_context = unsafe { v8::Local::new(scope, &*context_ptr) };
                        let registered_context = v8::Global::new(scope.as_ref(), local_context);
                        page_inspector.attach_isolated_context(
                            inspector_document_isolate,
                            inspector,
                            local_context,
                            registered_context,
                            Some(replaced_registration_id),
                            &name,
                            grant_universal_access,
                            inspector_frame_id,
                        )
                    },
                )?;
            if let Some((inspector_context, registration_id)) = inspector_context {
                if let Some(world) = self
                    .page_isolated_world_contexts
                    .context_mut(execution_context_id)
                {
                    world.inspector_context_registration_id = registration_id;
                }
                self.page_isolated_world_contexts
                    .set_inspector_execution_context_id(
                        execution_context_id,
                        inspector_context.id,
                        inspector_context.unique_id,
                    );
            }
        }
        Ok(())
    }

    pub(crate) fn create_isolated_world(
        &mut self,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        self.ensure_isolated_world_for_owner(None, name, grant_universal_access)
    }

    pub(crate) fn create_isolated_world_for_frame(
        &mut self,
        frame_id: &str,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        self.ensure_isolated_world_for_frame_owner(None, frame_id, name, grant_universal_access)
    }

    pub(crate) fn ensure_isolated_world_for_owner(
        &mut self,
        devtools_session: Option<&DevToolsSessionKey>,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        if let Some(execution_context_id) = self
            .page_isolated_world_contexts
            .execution_context_id_for_scope(devtools_session, None, name)
        {
            let owner_is_current = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .is_some_and(|world| {
                    self._context_host
                        .borrow()
                        .document_task_owner_is_current(world.document_owner)
                });
            if !owner_is_current {
                return Err(anyhow!(
                    "isolated world `{name}` belongs to a retired main document"
                ));
            }
            return Ok(execution_context_id);
        }
        self.create_new_isolated_world(
            devtools_session.cloned(),
            name,
            grant_universal_access,
            None,
            None,
        )
    }

    pub(crate) fn ensure_isolated_world_for_frame_owner(
        &mut self,
        devtools_session: Option<&DevToolsSessionKey>,
        frame_id: &str,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        if let Some(execution_context_id) = self
            .page_isolated_world_contexts
            .execution_context_id_for_scope(devtools_session, Some(frame_id), name)
        {
            let owner_is_current = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .is_some_and(|world| {
                    self._context_host
                        .borrow()
                        .document_task_owner_is_current(world.document_owner)
                });
            if owner_is_current {
                return Ok(execution_context_id);
            }
            self.destroy_isolated_world_context(execution_context_id);
        }
        let child_handle = self
            ._context_host
            .borrow()
            .child_browsing_context_handle_by_frame_id(frame_id)
            .ok_or_else(|| anyhow!("no live child browsing context for frame `{frame_id}`"))?;
        self.create_new_isolated_world(
            devtools_session.cloned(),
            name,
            grant_universal_access,
            Some(frame_id.to_owned()),
            Some(child_handle),
        )
    }

    pub(crate) fn install_runtime_binding(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        if let Some(execution_context_id) = execution_context_id {
            return self.install_runtime_binding_in_execution_context(execution_context_id, name);
        }
        let Some(execution_context_name) = execution_context_name else {
            return self.install_runtime_binding_in_default_context(name);
        };
        let matching_context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids_for_name(execution_context_name);
        if matching_context_ids.is_empty() {
            return Ok(());
        }

        for execution_context_id in matching_context_ids {
            self.install_runtime_binding_in_isolated_context(execution_context_id, name)?;
        }
        Ok(())
    }

    pub(crate) fn remove_runtime_binding(&mut self, name: &str) -> Result<()> {
        self.remove_runtime_binding_from_default_context(name)?;
        let isolated_context_ids = self
            .page_isolated_world_contexts
            .execution_context_ids()
            .collect::<Vec<_>>();
        for execution_context_id in isolated_context_ids {
            self.remove_runtime_binding_from_isolated_context(execution_context_id, name)?;
        }
        self.remove_runtime_binding_from_child_default_contexts(name)?;
        Ok(())
    }

    pub(crate) fn remove_default_runtime_binding(&mut self, name: &str) -> Result<()> {
        self.remove_runtime_binding_from_default_context(name)
    }

    pub(crate) fn run_document_start_script_now(
        &mut self,
        script: &DocumentStartScript,
    ) -> Result<Option<(i64, bool)>> {
        match script.world_name.as_deref() {
            Some(world_name) => {
                let created = self
                    .page_isolated_world_contexts
                    .execution_context_id_for_scope(
                        script.devtools_session.as_ref(),
                        None,
                        world_name,
                    )
                    .is_none();
                let execution_context_id = self.ensure_isolated_world_for_owner(
                    script.devtools_session.as_ref(),
                    world_name,
                    false,
                )?;
                self.exec_in_execution_context(execution_context_id, &script.source)?;
                Ok(Some((execution_context_id, created)))
            }
            None => {
                self.exec_runtime_turn(&script.source, None)?;
                Ok(None)
            }
        }
    }

    pub(crate) fn run_document_start_script_in_execution_context(
        &mut self,
        execution_context_id: i64,
        script: &DocumentStartScript,
    ) -> Result<()> {
        self.exec_in_execution_context(execution_context_id, &script.source)
    }
}
