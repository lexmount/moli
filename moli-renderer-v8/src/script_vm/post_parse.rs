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
    pub(super) fn new_main_default_in_scope<'s>(
        scope: &mut v8::PinScope<'s, '_, ()>,
        global_template: v8::Local<'s, v8::ObjectTemplate>,
        context_host: Rc<RefCell<JsContextHost>>,
        resource_owner_id: ResourceOwnerId,
        promise_reject_dispatch: &PromiseRejectDispatchSlot,
        storage_bucket_store: SharedStorageBucketStore,
        environment: crate::script_vm::RendererPageScriptEnvironment,
    ) -> Result<Self> {
        Self::new_in_scope(
            scope,
            global_template,
            context_host,
            resource_owner_id,
            promise_reject_dispatch,
            None,
            Some(storage_bucket_store),
            WindowContextBootstrapMode::MainDefault,
            Some(environment),
            false,
        )
    }

    pub(super) fn new_main_default(
        isolate: &mut v8::OwnedIsolate,
        isolate_bootstrap: &IsolateBootstrapCache,
        context_host: Rc<RefCell<JsContextHost>>,
        resource_owner_id: ResourceOwnerId,
        promise_reject_dispatch: &PromiseRejectDispatchSlot,
        indexed_db_manager: Option<WeakIndexedDbManager>,
        storage_bucket_store: Option<SharedStorageBucketStore>,
        renderer_page_script_environment: Option<crate::script_vm::RendererPageScriptEnvironment>,
        reuse_main_window_proxy: bool,
    ) -> Result<Self> {
        Self::new_with_mode(
            isolate,
            isolate_bootstrap,
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
                    .with_main_window_proxy(|proxy| v8::Local::new(scope, proxy))?,
            ),
            WindowContextBootstrapMode::MainDefault => None,
            WindowContextBootstrapMode::ChildDefault { child_handle, .. } => {
                unsafe { &mut *host_ptr }
                    .take_child_window_proxy_shell_for_realm(scope, child_handle)
            }
            WindowContextBootstrapMode::PopupDefault { popup_id, .. } => {
                unsafe { &mut *host_ptr }.take_popup_window_proxy_for_realm(scope, popup_id)
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
        let runtime_observable_context_token =
            unsafe { &mut *host_ptr }.allocate_runtime_observable_context_token();
        install_runtime_observable_context_token_for_context(
            local_context,
            runtime_observable_context_token,
        );
        let mut runtime = super::WindowRealmRuntime::new(
            scope,
            local_context,
            Rc::downgrade(&context_host),
            resource_owner_id,
            runtime_observable_context_token,
            unsafe { &*host_ptr }.deferred_context_host_release_queue(),
            matches!(mode, WindowContextBootstrapMode::Isolated { .. }),
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
            WindowContextBootstrapMode::PopupDefault { popup_id, .. } => {
                unsafe { &*host_ptr }.popup_default_world_security_token_key(popup_id)
            }
            WindowContextBootstrapMode::Isolated {
                child_handle: Some(child_handle),
                ..
            } => unsafe { &*host_ptr }.child_isolated_world_security_token_key(child_handle),
            WindowContextBootstrapMode::Isolated {
                child_handle: None, ..
            } => unsafe { &*host_ptr }.main_isolated_world_security_token_key(),
        };
        let dispatch_scope = match mode {
            WindowContextBootstrapMode::PopupDefault { popup_id, .. } => {
                crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id)
            }
            WindowContextBootstrapMode::ChildDefault { child_handle, .. }
            | WindowContextBootstrapMode::Isolated {
                child_handle: Some(child_handle),
                ..
            } => crate::native_bridge::OwnerDispatchScope::Child(child_handle),
            _ => crate::native_bridge::OwnerDispatchScope::Top,
        };
        let access_policy = match mode {
            WindowContextBootstrapMode::Isolated { access_policy, .. } => access_policy,
            _ => crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin,
        };
        unsafe { &*host_ptr }.install_window_context_security_origin(
            scope,
            local_context,
            dispatch_scope,
            access_policy,
        );
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
        if matches!(
            mode,
            WindowContextBootstrapMode::MainDefault
                | WindowContextBootstrapMode::ChildDefault { .. }
                | WindowContextBootstrapMode::PopupDefault { .. }
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
            WindowContextBootstrapMode::PopupDefault { popup_id, .. } => {
                unsafe { &*host_ptr }.bind_popup_window_context_owner_before_runtime_bootstrap(
                    scope,
                    bootstrap_global,
                    popup_id,
                )?;
            }
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
        runtime.install_bridge_ref(bridge_ref);
        let mut realm_registration = PendingWindowRealmBootstrapRegistration::register(
            host_ptr,
            mode,
            runtime_observable_context_token,
            resource_owner_id,
        )?;
        if let WindowContextBootstrapMode::PopupDefault {
            popup_id,
            local_window_id,
        } = mode
        {
            unsafe { &mut *host_ptr }.register_window_execution_context(
                crate::native_bridge::WindowExecutionContextBinding::new(
                    crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
                        popup_id,
                        local_window_id,
                    },
                    dispatch_scope,
                    runtime_observable_context_token,
                    v8::Global::new(scope, local_context),
                ),
            );
        }
        // SecureContext is origin-based, not document-URL-based. Initial
        // about:blank/srcdoc child contexts can keep about:* document URLs while
        // inheriting the creator's origin, so child bootstrap must ask the
        // context host for the origin-aware URL instead of using current_url().
        let secure_context_available = match mode {
            WindowContextBootstrapMode::PopupDefault { popup_id, .. } => {
                unsafe { &*host_ptr }.popup_secure_context_available(popup_id)
            }
            WindowContextBootstrapMode::Isolated {
                child_handle: Some(child_handle),
                ..
            }
            | WindowContextBootstrapMode::ChildDefault { child_handle, .. } => {
                moli_url::is_potentially_trustworthy_url(
                    &unsafe { &*host_ptr }
                        .child_browsing_context_secure_context_url(child_handle)
                        .unwrap_or_else(|| unsafe { &*host_ptr }.document_url().clone()),
                )
            }
            WindowContextBootstrapMode::MainDefault
            | WindowContextBootstrapMode::Isolated {
                child_handle: None, ..
            } => moli_url::is_potentially_trustworthy_url(
                &unsafe { &*host_ptr }.main_document_secure_context_url(),
            ),
        };
        if let WindowContextBootstrapMode::Isolated { child_handle, .. } = mode {
            let owner_context = match child_handle {
                Some(handle) => unsafe { &mut *host_ptr }
                    .ensure_prebootstrapped_child_default_context(scope, handle)?,
                None => unsafe { &*host_ptr }
                    .page_default_context(scope)
                    .ok_or_else(|| {
                        anyhow::anyhow!("isolated History requires the default Window")
                    })?,
            };
            let owner = owner_context.global(scope);
            crate::context_bootstrap::bind_isolated_window_history_owner(scope, global, owner);
        }
        finish_context_bootstrap(scope, unsafe { &mut *host_ptr }, secure_context_available)?;
        match mode {
            WindowContextBootstrapMode::PopupDefault { popup_id, .. } => {
                unsafe { &mut *host_ptr }
                    .configure_popup_default_world_global(scope, global, popup_id)?;
            }
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
            WindowContextBootstrapMode::MainDefault
            | WindowContextBootstrapMode::Isolated {
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
        // Child Window exposure can reach bootstrap while the caller holds
        // the host's RefCell borrow. Follow the bootstrap's raw host access
        // discipline and end each native read before touching V8 again.
        if unsafe { &*host_ptr }.document_host_is_published() {
            let queue = unsafe { &*host_ptr }.deferred_context_host_release_queue();
            crate::util::retain_context_host_for_document_realm(local_context, context_host, queue);
        }
        Ok(Self { runtime })
    }

    pub(super) fn into_runtime(self) -> super::WindowRealmRuntime {
        self.runtime
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
) -> Result<super::WindowRealmRuntime> {
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
    Ok(context_bootstrap.into_runtime())
}

pub(crate) fn bootstrap_popup_default_context_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    global_template: v8::Local<'s, v8::ObjectTemplate>,
    context_host: Rc<RefCell<JsContextHost>>,
    resource_owner_id: ResourceOwnerId,
    promise_reject_dispatch: &PromiseRejectDispatchSlot,
    indexed_db_manager: Option<WeakIndexedDbManager>,
    storage_bucket_store: Option<SharedStorageBucketStore>,
    popup_id: u64,
    local_window_id: crate::window_document_identity::LightweightPopupLocalWindowId,
) -> Result<super::WindowRealmRuntime> {
    let context_bootstrap = ScriptVmContextBootstrap::new_in_scope(
        scope,
        global_template,
        context_host,
        resource_owner_id,
        promise_reject_dispatch,
        indexed_db_manager,
        storage_bucket_store,
        WindowContextBootstrapMode::PopupDefault {
            popup_id,
            local_window_id,
        },
        None,
        false,
    )?;
    Ok(context_bootstrap.into_runtime())
}

#[derive(Clone, Copy, Debug)]
enum WindowContextBootstrapMode {
    MainDefault,
    PopupDefault {
        popup_id: u64,
        local_window_id: crate::window_document_identity::LightweightPopupLocalWindowId,
    },
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
    resource_owner_id: ResourceOwnerId,
    committed: bool,
}

impl PendingWindowRealmBootstrapRegistration {
    fn register(
        host: *mut JsContextHost,
        mode: WindowContextBootstrapMode,
        realm_token: crate::native_bridge::RuntimeObservableContextToken,
        resource_owner_id: ResourceOwnerId,
    ) -> Result<Option<Self>> {
        let Some((owner, dispatch_scope, access_policy)) = mode.registration(unsafe { &*host })
        else {
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
        Ok(Some(Self {
            host,
            realm_token,
            resource_owner_id,
            committed: false,
        }))
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for PendingWindowRealmBootstrapRegistration {
    fn drop(&mut self) {
        if !self.committed {
            unsafe { &mut *self.host }.retire_window_execution_contexts_for_context_token(
                self.realm_token,
                Some(self.resource_owner_id),
            );
        }
    }
}

impl WindowContextBootstrapMode {
    fn registration(
        self,
        host: &JsContextHost,
    ) -> Option<(
        crate::native_bridge::WindowExecutionContextOwner,
        crate::native_bridge::OwnerDispatchScope,
        crate::native_bridge::WindowExecutionContextAccessPolicy,
    )> {
        match self {
            Self::MainDefault => Some((
                host.current_window_execution_context_owner(
                    crate::native_bridge::OwnerDispatchScope::Top,
                )?,
                crate::native_bridge::OwnerDispatchScope::Top,
                crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            )),
            Self::PopupDefault {
                popup_id,
                local_window_id,
            } => Some((
                crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
                    popup_id,
                    local_window_id,
                },
                crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id),
                crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            )),
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
