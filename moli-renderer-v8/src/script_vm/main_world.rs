use super::*;

pub(super) enum MainWorld {
    // The large startup seeds are released when the realm is materialized.
    Pending(Box<PendingMainWorld>),
    Ready {
        runtime: WindowRealmRuntime,
        _baseline_globals: ScriptGlobalsBaseline,
    },
}

impl MainWorld {
    pub(super) fn current(&self) -> Option<&WindowRealmRuntime> {
        match self {
            Self::Pending(_) => None,
            Self::Ready { runtime, .. } => Some(runtime),
        }
    }

    pub(super) fn current_mut(&mut self) -> Option<&mut WindowRealmRuntime> {
        match self {
            Self::Pending(_) => None,
            Self::Ready { runtime, .. } => Some(runtime),
        }
    }

    pub(super) fn pending(&self) -> Option<&PendingMainWorld> {
        match self {
            Self::Pending(pending) => Some(pending),
            Self::Ready { .. } => None,
        }
    }

    pub(super) fn pending_mut(&mut self) -> Option<&mut PendingMainWorld> {
        match self {
            Self::Pending(pending) => Some(pending),
            Self::Ready { .. } => None,
        }
    }
}

/// Native Document state is already live. These are the remaining inputs for
/// its first main-world Context, not a second Document or bootstrap DOM.
#[derive(Default)]
pub(super) struct PendingMainWorld {
    pub(super) inherited_security_token: Option<v8::Global<v8::Value>>,
    pub(super) navigator_identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    pub(super) history_position: Option<moli_session_history::SessionHistoryPosition>,
    pub(super) history_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
}

pub(super) fn initialize(
    isolate: &RendererIsolateHandle,
    host: &Rc<RefCell<PageScriptHost>>,
    resource_owner_id: crate::resource_owner::ResourceOwnerId,
    promise_reject_dispatch: &PromiseRejectDispatchSlot,
    storage_bucket_store: crate::context_bootstrap::SharedStorageBucketStore,
    indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    environment: Option<RendererPageScriptEnvironment>,
    root_frame_id: Option<&str>,
    inspector: &mut DocumentInspectorBinding,
    pending: &PendingMainWorld,
) -> Result<(WindowRealmRuntime, ScriptGlobalsBaseline)> {
    // An interrupted bootstrap can retain the proxy while the main world is
    // still Pending. Derive reuse from the live environment on every attempt.
    let reuse_main_window_proxy = environment
        .as_ref()
        .is_some_and(RendererPageScriptEnvironment::has_main_window_proxy);
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
                reuse_main_window_proxy,
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
        let timing = host.borrow().document_timing_snapshot();
        crate::context_bootstrap::bind_document_performance(scope, global, timing)?;
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
    register_main_window_execution_context_for_bootstrap(isolate, host, &runtime.context)?;
    let url = host.borrow().document_url().clone();
    let baseline = isolate.with_renderer_document_isolate_and_inspector_mut(|v8, backend| {
        ScriptVmDefaultWorldBootstrap::capture_baseline_globals_and_attach_context(
            isolate.clone(),
            v8,
            backend,
            inspector,
            &runtime.context,
            &url,
            root_frame_id,
        )
    })?;
    Ok((runtime, baseline))
}

impl ScriptVm {
    pub(super) fn current_main_world(&self) -> Option<&WindowRealmRuntime> {
        self.main_world.current()
    }

    pub(crate) fn ensure_main_world(&mut self) -> Result<&WindowRealmRuntime> {
        if let Some(pending) = self.main_world.pending() {
            let (runtime, baseline_globals) = initialize(
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
            self.renderer_document_isolate
                .with_renderer_document_isolate_mut(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    self.retain_native_hosts_in_scope(&mut scope.init(), &runtime);
                });
            self.main_world = MainWorld::Ready {
                runtime,
                _baseline_globals: baseline_globals,
            };
        }
        self.current_main_world()
            .ok_or_else(|| anyhow!("main-world initialization did not produce a Window realm"))
    }
}
