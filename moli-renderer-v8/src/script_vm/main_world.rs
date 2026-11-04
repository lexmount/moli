use super::*;

/// Native Document state is already live. These are the remaining inputs for
/// its first main-world Context, not a second Document or bootstrap DOM.
#[derive(Default)]
pub(super) struct PendingMainWorld {
    pub(super) inherited_security_token: Option<v8::Global<v8::Value>>,
    pub(super) reuse_main_window_proxy: bool,
    pub(super) navigator_identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    pub(super) history_position: Option<moli_session_history::SessionHistoryPosition>,
    pub(super) history_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
    pub(super) performance: DeferredMainWorldPerformance,
}

pub(super) struct DeferredMainWorldPerformance {
    origin: f64,
    started: std::time::Instant,
    timestamps: [Option<f64>; 4],
}

impl Default for DeferredMainWorldPerformance {
    fn default() -> Self {
        Self {
            origin: crate::context_bootstrap::unix_epoch_millis(),
            started: std::time::Instant::now(),
            timestamps: [None; 4],
        }
    }
}

impl DeferredMainWorldPerformance {
    pub(super) fn record(&mut self, index: usize) {
        let previous = self.timestamps[..index]
            .iter()
            .flatten()
            .copied()
            .fold(0.0, f64::max);
        self.timestamps[index] =
            Some((self.started.elapsed().as_secs_f64() * 1000.0).max(previous + 0.001));
    }
}

pub(super) fn initialize(
    isolate: &RendererDocumentIsolateHandle,
    host: &Rc<RefCell<JsContextHost>>,
    resource_owner_id: crate::resource_owner::ResourceOwnerId,
    promise_reject_dispatch: &PromiseRejectDispatchSlot,
    storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
    indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    environment: Option<RendererPageScriptEnvironment>,
    root_frame_id: Option<&str>,
    inspector: &mut DocumentInspectorBinding,
    pending: &PendingMainWorld,
) -> Result<(WindowRealmRuntime, ScriptGlobalsBaseline)> {
    let runtime = isolate
        .with_entered_renderer_document_isolate_and_bootstrap(|isolate, assets| {
            ScriptVmContextBootstrap::new_main_default(
                isolate,
                assets,
                host.clone(),
                resource_owner_id,
                promise_reject_dispatch,
                indexed_db_manager,
                Some(storage_bucket_store),
                environment,
                pending.reuse_main_window_proxy,
            )
        })?
        .into_runtime();
    isolate.with_entered_renderer_document_isolate(|isolate| {
        let scope = pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let context = v8::Local::new(scope, &runtime.context);
        host.borrow_mut()
            .install_page_default_context(scope, context);
        if let Some(token) = pending.inherited_security_token.as_ref()
            && !host.borrow().document_sandbox_policy().forces_opaque_origin
        {
            context.set_security_token(v8::Local::new(scope, token));
        }
        let scope = &mut v8::ContextScope::new(scope, context);
        let origin = {
            let host = host.borrow();
            if host.document_sandbox_policy().forces_opaque_origin {
                "null".to_owned()
            } else {
                host.current_main_document_resource_loader()
                    .expect("Document has a resource authority before its first script")
                    .fetch_context()
                    .origin()
                    .to_owned()
            }
        };
        let global = context.global(scope);
        crate::context_bootstrap::set_window_origin_runtime_state(scope, global, &origin)?;
        crate::context_bootstrap::initialize_main_session_history(scope);
        crate::context_bootstrap::bind_deferred_main_world_performance(
            scope,
            global,
            pending.performance.origin,
            pending.performance.timestamps,
        )?;
        if let Some(identity) = pending.navigator_identity.as_ref() {
            crate::context_bootstrap::set_window_navigator_identity(scope, identity)?;
        }
        if let Some(seed) = pending.history_entry.as_ref() {
            crate::context_bootstrap::install_navigation_bootstrap_entry(scope, seed);
        } else if let Some(position) = pending.history_position {
            crate::context_bootstrap::install_session_history_position(scope, position);
        }
        // Frame discovery also runs natively while the main world is absent.
        let host = unsafe { &mut *host.as_ptr() };
        host.sync_child_browsing_context_subtree(scope, host.document_handle());
        Ok(())
    })?;
    let url = host.borrow().document_url().clone();
    let baseline = isolate.with_renderer_document_isolate_and_inspector_mut(|v8, backend| {
        ScriptVmDefaultWorldBootstrap::attach_context_and_capture_baseline_globals(
            isolate.clone(),
            v8,
            backend,
            inspector,
            &runtime.context,
            &url,
            root_frame_id,
        )
    })?;
    register_main_window_execution_context_for_bootstrap(isolate, host, &runtime.context)?;
    Ok((runtime, baseline))
}

impl ScriptVm {
    pub(super) fn initialize_main_world_for_window_handler(&mut self) -> Result<()> {
        if self.main_world_is_initialized() {
            return Ok(());
        }
        let dom = self.document_runtime.dom_host().dom();
        let has_handler = dom
            .body_node_id()
            .and_then(|body| dom.node(body))
            .and_then(|node| node.as_element())
            .is_some_and(|body| {
                ["onload", "onpageshow"]
                    .iter()
                    .any(|name| body.attribute(name).is_some())
            });
        if has_handler {
            self.initialize_main_world_if_needed()?;
        }
        Ok(())
    }

    pub(crate) fn main_world_is_initialized(&self) -> bool {
        self.page_default_runtime.is_initialized()
    }

    pub(crate) fn initialize_main_world_if_needed(&mut self) -> Result<()> {
        let Some(pending) = self.pending_main_world.as_ref() else {
            return Ok(());
        };
        let (runtime, baseline) = initialize(
            &self.renderer_document_isolate,
            &self._context_host,
            self.resource_owner_id,
            &self.promise_reject_dispatch,
            self.storage_bucket_store.clone(),
            self.indexed_db_manager.clone(),
            self.renderer_page_script_environment.clone(),
            self.root_frame_id.as_deref(),
            &mut self.page_inspector,
            pending,
        )?;
        self.page_default_runtime = runtime;
        self.baseline_globals = baseline;
        self.pending_main_world = None;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                self.retain_native_hosts_in_scope(&mut scope.init());
            });
        Ok(())
    }
}
