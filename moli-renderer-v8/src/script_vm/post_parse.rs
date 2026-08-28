use super::{IsolateBootstrapCache, ScriptVmContextBootstrap};
use crate::{
    context_bootstrap::{
        SharedStorageBucketStore, WeakIndexedDbManager, set_indexed_db_manager_for_context,
        set_storage_bucket_store_for_context,
    },
    context_bootstrap::{finish_context_bootstrap, install_console_message_buffers_for_context},
    document_runtime::{DeferredPageTaskLane, DocumentRuntime, DomHandle},
    host::ScriptHandleSource,
    host_bindings::install_host_bindings,
    native_bridge::{JsContextHost, install_runtime_observable_context_token_for_context},
    planning::PreparedScript,
    resource_owner::{ResourceOwnerId, install_resource_owner_for_context},
    script_vm::runtime_bindings::{
        PromiseRejectDispatchSlot, install_promise_reject_dispatch_for_context,
    },
    types::{ScriptKind, ScriptSourceKind},
    util::set_private_value,
    window_host,
};
use anyhow::Result;
use std::{cell::RefCell, pin::pin, rc::Rc};

pub(super) fn dynamic_script_execute_is_runnable_before_dom_content_loaded(
    document_runtime: &DocumentRuntime,
    script: &PreparedScript,
) -> bool {
    if script.kind == ScriptKind::ImportMap && script.source_kind == ScriptSourceKind::Inline {
        return true;
    }
    !document_runtime.prepared_script_waits_until_dom_content_loaded(script)
        || script.host_script_handle.as_deref().is_some_and(|handle| {
            document_runtime.script_handle_source(handle) == ScriptHandleSource::DocumentWriteOwned
                && document_runtime.script_handle_followup_lane(handle)
                    == Some(DeferredPageTaskLane::PreDomContentLoaded)
        })
}

impl ScriptVmContextBootstrap {
    /// Creates a main default world inside a V8 scope that is already entered.
    ///
    /// Native callbacks such as `window.open()` cannot borrow the owning
    /// document isolate a second time while author script is on the stack.
    /// Keeping the in-scope primitive next to the child-default equivalent
    /// lets a caller prebootstrap a distinct Page realm without re-entering
    /// the isolate; Inspector materialization remains a later owner action.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_main_default_in_scope<'s>(
        scope: &mut v8::PinScope<'s, '_, ()>,
        global_template: v8::Local<'s, v8::ObjectTemplate>,
        context_host: Rc<RefCell<JsContextHost>>,
        resource_owner_id: ResourceOwnerId,
        promise_reject_dispatch: &PromiseRejectDispatchSlot,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        renderer_page_script_environment: Option<crate::script_vm::RendererPageScriptEnvironment>,
        reuse_main_window_proxy: bool,
    ) -> Result<Self> {
        Self::new_in_scope(
            scope,
            global_template,
            context_host,
            resource_owner_id,
            promise_reject_dispatch,
            indexed_db_manager,
            storage_bucket_store,
            WindowContextBootstrapMode::MainDefault,
            renderer_page_script_environment,
            reuse_main_window_proxy,
        )
    }

    pub(super) fn new_isolated(
        isolate: &mut v8::OwnedIsolate,
        isolate_bootstrap: &IsolateBootstrapCache,
        context_host: Rc<RefCell<JsContextHost>>,
        resource_owner_id: ResourceOwnerId,
        promise_reject_dispatch: &PromiseRejectDispatchSlot,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        child_handle: Option<DomHandle>,
        expected_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        access_policy: crate::native_bridge::WindowExecutionContextAccessPolicy,
    ) -> Result<Self> {
        Self::new_with_mode(
            isolate,
            isolate_bootstrap,
            context_host,
            resource_owner_id,
            promise_reject_dispatch,
            indexed_db_manager,
            storage_bucket_store,
            WindowContextBootstrapMode::Isolated {
                child_handle,
                expected_owner,
                access_policy,
            },
            None,
            false,
        )
    }

    pub(super) fn new_child_default(
        isolate: &mut v8::OwnedIsolate,
        isolate_bootstrap: &IsolateBootstrapCache,
        context_host: Rc<RefCell<JsContextHost>>,
        resource_owner_id: ResourceOwnerId,
        promise_reject_dispatch: &PromiseRejectDispatchSlot,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        child_handle: DomHandle,
        expected_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Result<Self> {
        Self::new_with_mode(
            isolate,
            isolate_bootstrap,
            context_host,
            resource_owner_id,
            promise_reject_dispatch,
            indexed_db_manager,
            storage_bucket_store,
            WindowContextBootstrapMode::ChildDefault {
                child_handle,
                expected_owner,
            },
            None,
            false,
        )
    }

    fn new_with_mode(
        isolate: &mut v8::OwnedIsolate,
        isolate_bootstrap: &IsolateBootstrapCache,
        context_host: Rc<RefCell<JsContextHost>>,
        resource_owner_id: ResourceOwnerId,
        promise_reject_dispatch: &PromiseRejectDispatchSlot,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        mode: WindowContextBootstrapMode,
        renderer_page_script_environment: Option<crate::script_vm::RendererPageScriptEnvironment>,
        reuse_main_window_proxy: bool,
    ) -> Result<Self> {
        let scope = pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let global_template = isolate_bootstrap.global_template(scope);
        Self::new_in_scope(
            scope,
            global_template,
            context_host,
            resource_owner_id,
            promise_reject_dispatch,
            indexed_db_manager,
            storage_bucket_store,
            mode,
            renderer_page_script_environment,
            reuse_main_window_proxy,
        )
    }

    fn new_in_scope<'s>(
        scope: &mut v8::PinScope<'s, '_, ()>,
        global_template: v8::Local<'s, v8::ObjectTemplate>,
        context_host: Rc<RefCell<JsContextHost>>,
        resource_owner_id: ResourceOwnerId,
        promise_reject_dispatch: &PromiseRejectDispatchSlot,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        mode: WindowContextBootstrapMode,
        renderer_page_script_environment: Option<crate::script_vm::RendererPageScriptEnvironment>,
        reuse_main_window_proxy: bool,
    ) -> Result<Self> {
        let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
        let reusable_window_proxy = match mode {
            WindowContextBootstrapMode::MainDefault if reuse_main_window_proxy => Some(
                renderer_page_script_environment
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "replacement main context is missing its page script environment"
                        )
                    })?
                    .take_main_window_proxy_for_context(scope)?,
            ),
            WindowContextBootstrapMode::MainDefault => None,
            WindowContextBootstrapMode::ChildDefault { child_handle, .. } => {
                unsafe { &mut *host_ptr }
                    .take_child_window_proxy_shell_for_realm(scope, child_handle)
            }
            WindowContextBootstrapMode::Isolated { .. } => None,
        };
        let local_context = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_template: Some(global_template),
                global_object: reusable_window_proxy.map(Into::into),
                ..Default::default()
            },
        );
        local_context.set_allow_generation_from_strings(false);
        if let Some(reusable_window_proxy) = reusable_window_proxy
            && !local_context
                .global(scope)
                .strict_equals(reusable_window_proxy.into())
        {
            return Err(anyhow::anyhow!(
                "V8 did not attach the committed main WindowProxy to the replacement context"
            ));
        }
        if matches!(mode, WindowContextBootstrapMode::MainDefault)
            && reusable_window_proxy.is_none()
            && let Some(environment) = renderer_page_script_environment.as_ref()
        {
            environment.install_initial_main_window_proxy(v8::Global::new(
                scope,
                local_context.global(scope),
            ))?;
        }
        if matches!(mode, WindowContextBootstrapMode::MainDefault)
            && reusable_window_proxy.is_some()
        {
            tracing::debug!("attached replacement main context to stable WindowProxy");
        }
        let security_token_key = match mode {
            WindowContextBootstrapMode::MainDefault => {
                unsafe { &*host_ptr }.main_default_world_security_token_key()
            }
            WindowContextBootstrapMode::ChildDefault { child_handle, .. } => {
                unsafe { &*host_ptr }.child_default_world_security_token_key(child_handle)
            }
            WindowContextBootstrapMode::Isolated {
                child_handle: Some(child_handle),
                ..
            } => unsafe { &*host_ptr }.child_isolated_world_security_token_key(child_handle),
            WindowContextBootstrapMode::Isolated {
                child_handle: None, ..
            } => unsafe { &*host_ptr }.main_isolated_world_security_token_key(),
        };
        if !crate::native_bridge::set_window_security_token(
            scope,
            local_context,
            security_token_key.as_deref(),
        ) {
            tracing::warn!(
                ?mode,
                "failed to allocate internalized Window security token; using unique context token"
            );
        }
        if matches!(mode, WindowContextBootstrapMode::MainDefault)
            && reusable_window_proxy.is_some()
            && let Some(security_token) = renderer_page_script_environment
                .as_ref()
                .and_then(|environment| environment.take_initial_main_window_security_token(scope))
        {
            // Initial about:blank inherits its creator's exact effective
            // origin. Reusing the creator-visible proxy is insufficient for
            // opaque or document.domain-mutated origins because V8 otherwise
            // assigns the new context a distinct default token.
            local_context.set_security_token(security_token);
        }
        if matches!(
            mode,
            WindowContextBootstrapMode::MainDefault
                | WindowContextBootstrapMode::ChildDefault { .. }
        ) {
            unsafe { &*host_ptr }.install_default_world_wrapper_cache_for_context(local_context);
        }
        install_resource_owner_for_context(local_context, resource_owner_id);
        install_promise_reject_dispatch_for_context(local_context, promise_reject_dispatch);
        set_indexed_db_manager_for_context(local_context, indexed_db_manager);
        set_storage_bucket_store_for_context(local_context, storage_bucket_store);
        install_console_message_buffers_for_context(local_context);
        let scope = &mut v8::ContextScope::new(scope, local_context);
        let bootstrap_global = local_context.global(scope);
        // Window-owned accessors installed below must resolve against this
        // context's LocalWindow from their first observable bootstrap call.
        match mode {
            WindowContextBootstrapMode::ChildDefault { child_handle, .. }
            | WindowContextBootstrapMode::Isolated {
                child_handle: Some(child_handle),
                ..
            } => unsafe { &*host_ptr }.bind_child_window_context_owner_before_runtime_bootstrap(
                scope,
                bootstrap_global,
                child_handle,
            )?,
            WindowContextBootstrapMode::MainDefault
            | WindowContextBootstrapMode::Isolated {
                child_handle: None, ..
            } => {}
        }
        // Use RefCell::as_ptr to get a raw pointer without holding a borrow guard so that
        // V8 callbacks invoked during bootstrap can borrow_mut() without contention.
        install_host_bindings(scope, unsafe { &mut *host_ptr })?;
        let global = scope.get_current_context().global(scope);
        let bridge_ref = JsContextHost::install_into_bridge(&context_host, scope, global)?;
        if matches!(mode, WindowContextBootstrapMode::MainDefault)
            && let Some(environment) = renderer_page_script_environment.as_ref()
        {
            environment.bind_current_main_default_context(v8::Global::new(scope, local_context));
        }
        let runtime_observable_context_token =
            unsafe { &mut *host_ptr }.allocate_runtime_observable_context_token();
        install_runtime_observable_context_token_for_context(
            local_context,
            runtime_observable_context_token,
        );
        let mut realm_registration = PendingWindowRealmBootstrapRegistration::register(
            host_ptr,
            mode,
            runtime_observable_context_token,
            local_context,
        )?;
        // SecureContext is origin-based, not document-URL-based. Initial
        // about:blank/srcdoc child contexts can keep about:* document URLs while
        // inheriting the creator's origin, so child bootstrap must ask the
        // context host for the origin-aware URL instead of using current_url().
        let secure_context_url = match mode {
            WindowContextBootstrapMode::Isolated {
                child_handle: Some(child_handle),
                ..
            }
            | WindowContextBootstrapMode::ChildDefault { child_handle, .. } => {
                unsafe { &*host_ptr }
                    .child_browsing_context_secure_context_url(child_handle)
                    .unwrap_or_else(|| unsafe { &*host_ptr }.document_url().clone())
            }
            WindowContextBootstrapMode::MainDefault
            | WindowContextBootstrapMode::Isolated {
                child_handle: None, ..
            } => unsafe { &*host_ptr }.document_url().clone(),
        };
        finish_context_bootstrap(scope, unsafe { &mut *host_ptr }, &secure_context_url)?;
        if matches!(mode, WindowContextBootstrapMode::MainDefault)
            && let Some(environment) = renderer_page_script_environment.as_ref()
        {
            if reusable_window_proxy.is_some() {
                environment.restore_main_window_opener_after_navigation(scope, global);
            } else {
                let opener = unsafe { &mut *host_ptr }
                    .restore_current_top_level_opener_projection(scope, environment)?;
                set_private_value(
                    scope,
                    global,
                    crate::context_bootstrap::WINDOW_OPENER_SLOT,
                    opener,
                );
            }
            environment.restore_main_window_name_after_navigation(scope, global);
        }
        match mode {
            WindowContextBootstrapMode::Isolated {
                child_handle: Some(child_handle),
                expected_owner,
                access_policy,
            } => unsafe { &mut *host_ptr }.configure_child_isolated_world_global(
                scope,
                global,
                child_handle,
                expected_owner,
                runtime_observable_context_token,
                access_policy,
            )?,
            WindowContextBootstrapMode::ChildDefault {
                child_handle,
                expected_owner,
            } => {
                unsafe { &mut *host_ptr }.promote_child_window_proxy_shell_to_realm(
                    scope,
                    child_handle,
                    global,
                );
                unsafe { &mut *host_ptr }.configure_child_default_world_global(
                    scope,
                    global,
                    child_handle,
                    expected_owner,
                    runtime_observable_context_token,
                )?;
            }
            WindowContextBootstrapMode::MainDefault => {
                let top_window_endpoint = v8::Boolean::new(scope, true);
                set_private_value(
                    scope,
                    global,
                    window_host::TOP_WINDOW_MESSAGE_ENDPOINT_SLOT,
                    top_window_endpoint.into(),
                );
                let window_proxy_endpoint = renderer_page_script_environment.as_ref().map(
                    crate::script_vm::RendererPageScriptEnvironment::top_level_window_proxy_endpoint_id,
                );
                unsafe { &mut *host_ptr }
                    .install_top_level_window_proxy_cross_origin_access_surface(
                        scope,
                        global,
                        window_proxy_endpoint,
                    );
            }
            WindowContextBootstrapMode::Isolated {
                child_handle: None, ..
            } => {
                let top_window_endpoint = v8::Boolean::new(scope, true);
                set_private_value(
                    scope,
                    global,
                    window_host::TOP_WINDOW_MESSAGE_ENDPOINT_SLOT,
                    top_window_endpoint.into(),
                );
            }
        }
        if let Some(registration) = realm_registration.as_mut() {
            registration.commit();
        }
        crate::util::retain_context_host_for_document_realm(
            local_context,
            context_host,
            unsafe { &*host_ptr }.deferred_context_host_release_queue(),
        );
        Ok(Self {
            context: v8::Global::new(scope, local_context),
            runtime_observable_context_token,
            bridge_ref,
        })
    }

    pub(super) fn into_context_and_bridge_ref(
        self,
    ) -> (
        v8::Global<v8::Context>,
        crate::native_bridge::JsContextHostBridgeRef,
    ) {
        (self.context, self.bridge_ref)
    }
}

pub(crate) fn bootstrap_child_default_context_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    global_template: v8::Local<'s, v8::ObjectTemplate>,
    context_host: Rc<RefCell<JsContextHost>>,
    resource_owner_id: ResourceOwnerId,
    promise_reject_dispatch: &PromiseRejectDispatchSlot,
    indexed_db_manager: Option<WeakIndexedDbManager>,
    storage_bucket_store: Option<SharedStorageBucketStore>,
    child_handle: DomHandle,
    expected_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
) -> Result<(
    v8::Global<v8::Context>,
    crate::native_bridge::RuntimeObservableContextToken,
    crate::native_bridge::JsContextHostBridgeRef,
)> {
    let context_bootstrap = ScriptVmContextBootstrap::new_in_scope(
        scope,
        global_template,
        context_host,
        resource_owner_id,
        promise_reject_dispatch,
        indexed_db_manager,
        storage_bucket_store,
        WindowContextBootstrapMode::ChildDefault {
            child_handle,
            expected_owner,
        },
        None,
        false,
    )?;
    let runtime_observable_context_token = context_bootstrap.runtime_observable_context_token;
    let (context, bridge_ref) = context_bootstrap.into_context_and_bridge_ref();
    Ok((context, runtime_observable_context_token, bridge_ref))
}

#[derive(Clone, Copy, Debug)]
enum WindowContextBootstrapMode {
    MainDefault,
    ChildDefault {
        child_handle: DomHandle,
        expected_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    },
    Isolated {
        child_handle: Option<DomHandle>,
        expected_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        access_policy: crate::native_bridge::WindowExecutionContextAccessPolicy,
    },
}

struct PendingWindowRealmBootstrapRegistration {
    host: *mut JsContextHost,
    realm_token: crate::native_bridge::RuntimeObservableContextToken,
    committed: bool,
}

impl PendingWindowRealmBootstrapRegistration {
    fn register(
        host: *mut JsContextHost,
        mode: WindowContextBootstrapMode,
        realm_token: crate::native_bridge::RuntimeObservableContextToken,
        context: v8::Local<'_, v8::Context>,
    ) -> Result<Option<Self>> {
        let Some((owner, dispatch_scope, access_policy)) = mode.registration() else {
            return Ok(None);
        };
        if !unsafe { &mut *host }.register_window_execution_context_realm(
            owner,
            dispatch_scope,
            realm_token,
            access_policy,
        ) {
            anyhow::bail!("failed to register Window realm before runtime bootstrap");
        }
        let registration = Self {
            host,
            realm_token,
            committed: false,
        };
        let identity = crate::native_bridge::WindowExecutionContextIdentity::new(
            owner,
            dispatch_scope,
            realm_token,
            access_policy,
        );
        if !unsafe { &*host }.install_window_access_check_principal_for_context(context, identity) {
            anyhow::bail!("failed to install Window realm access-check principal");
        }
        Ok(Some(registration))
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for PendingWindowRealmBootstrapRegistration {
    fn drop(&mut self) {
        if !self.committed {
            unsafe { &mut *self.host }
                .retire_window_execution_contexts_for_context_token(self.realm_token);
        }
    }
}

impl WindowContextBootstrapMode {
    fn registration(
        self,
    ) -> Option<(
        crate::native_bridge::WindowExecutionContextOwner,
        crate::native_bridge::OwnerDispatchScope,
        crate::native_bridge::WindowExecutionContextAccessPolicy,
    )> {
        match self {
            Self::MainDefault => None,
            Self::ChildDefault {
                child_handle,
                expected_owner,
            } => Some((
                crate::native_bridge::WindowExecutionContextOwner::Frame(
                    expected_owner.local_window_id,
                ),
                crate::native_bridge::OwnerDispatchScope::Child(child_handle),
                crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            )),
            Self::Isolated {
                child_handle,
                expected_owner,
                access_policy,
            } => Some((
                crate::native_bridge::WindowExecutionContextOwner::Frame(
                    expected_owner.local_window_id,
                ),
                child_handle
                    .map(crate::native_bridge::OwnerDispatchScope::Child)
                    .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top),
                access_policy,
            )),
        }
    }
}
