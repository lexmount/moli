use super::super::{
    ChildDefaultContextBootstrapConfig, JsContextHost, PrebootstrappedChildDefaultContext,
    WeakPrebootstrappedChildDefaultContexts, WindowExecutionContextAccessPolicy,
    WindowExecutionContextBinding, WindowExecutionContextOwner,
};
use super::realm_state::{
    ChildWindowRealmInit, WindowWorldKind, initialize_child_window_realm_state,
};
use crate::{
    context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    native_bridge::{
        OwnerDispatchScope, RuntimeObservableContextToken,
        child_window_surface::bind_materialized_child_window_indexed_db_factory,
    },
    util::set_private_value,
};
use anyhow::Result;
use std::{cell::RefCell, rc::Weak};

impl JsContextHost {
    pub(crate) fn bind_child_window_indexed_db_factory_after_context_registration<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) {
        let global = scope.get_current_context().global(scope);
        bind_materialized_child_window_indexed_db_factory(scope, global, handle);
    }

    pub(crate) fn bind_child_window_context_owner_before_runtime_bootstrap(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        global: v8::Local<'_, v8::Object>,
        handle: DomHandle,
    ) -> Result<()> {
        self.current_child_document_task_owner(handle)
            .ok_or_else(|| anyhow::anyhow!("missing child LocalWindow owner"))?;
        set_private_value(
            scope,
            global,
            CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
            v8::Number::new(scope, handle.index() as f64).into(),
        );
        Ok(())
    }

    pub(crate) fn install_child_default_context_bootstrap(
        &mut self,
        host: Weak<RefCell<JsContextHost>>,
        pending_contexts: WeakPrebootstrappedChildDefaultContexts,
        resource_owner_id: crate::resource_owner::ResourceOwnerId,
        promise_reject_dispatch: crate::script_vm::PromiseRejectDispatchSlot,
    ) {
        self.child_default_context_bootstrap = Some(ChildDefaultContextBootstrapConfig {
            host,
            pending_contexts,
            resource_owner_id,
            promise_reject_dispatch,
        });
    }

    #[cfg(test)]
    pub(crate) fn force_child_default_context_preflight_failure_for_test(&mut self) {
        self.force_child_default_context_preflight_failure = true;
    }

    pub(crate) fn ensure_prebootstrapped_child_default_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Result<v8::Local<'s, v8::Context>> {
        #[cfg(test)]
        if self.force_child_default_context_preflight_failure {
            return Err(anyhow::anyhow!(
                "forced child default context preflight failure"
            ));
        }
        let owner = self
            .current_child_document_task_owner(handle)
            .ok_or_else(|| anyhow::anyhow!("missing child LocalWindow owner"))?;
        let execution_context_owner = WindowExecutionContextOwner::Frame(owner.local_window_id);
        let dispatch_scope = OwnerDispatchScope::Child(handle);
        if let Some((_, context)) =
            self.window_execution_context(scope, execution_context_owner, dispatch_scope)
        {
            self.frame_owner_store
                .ensure_child_realm(handle)
                .ok_or_else(|| anyhow::anyhow!("missing child realm owner record"))?;
            self.request_child_frame_realm_materialization(handle);
            return Ok(context);
        }
        let config = self
            .child_default_context_bootstrap
            .clone()
            .ok_or_else(|| anyhow::anyhow!("child default context bootstrap is unavailable"))?;
        let host = config
            .host
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("child default context host was retired"))?;
        let pending_contexts = config
            .pending_contexts
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("child default context owner was retired"))?;

        let retained_pending = {
            let contexts = pending_contexts.borrow();
            contexts.get(&handle).and_then(|context| {
                (context.local_window_id == owner.local_window_id).then(|| {
                    (
                        context.runtime_observable_context_token,
                        v8::Local::new(scope, &context.context),
                    )
                })
            })
        };
        if let Some((runtime_observable_context_token, context)) = retained_pending {
            self.register_window_execution_context(WindowExecutionContextBinding::new(
                execution_context_owner,
                dispatch_scope,
                runtime_observable_context_token,
                v8::Global::new(scope, context),
            ));
            let child_scope = &mut v8::ContextScope::new(scope, context);
            self.bind_child_window_indexed_db_factory_after_context_registration(
                child_scope,
                handle,
            );
            self.frame_owner_store
                .ensure_child_realm(handle)
                .ok_or_else(|| anyhow::anyhow!("missing child realm owner record"))?;
            self.request_child_frame_realm_materialization(handle);
            return Ok(context);
        }
        if let Some(stale) = pending_contexts.borrow_mut().remove(&handle) {
            self.retire_window_execution_contexts_for_context_token(
                stale.runtime_observable_context_token,
            );
            let stale_context = v8::Local::new(scope, &stale.context);
            {
                let stale_scope = &mut v8::ContextScope::new(scope, stale_context);
                crate::native_bridge::clear_context_embedder_state_for_teardown(stale_scope, false);
            }
            stale_context.detach_global();
        }

        let caller_global = scope.get_current_context().global(scope);
        let top = self.child_browsing_context_root_window(scope, handle, caller_global);
        let parent = self.child_browsing_context_parent_window(scope, handle, top);
        self.child_window_proxy_records
            .set_browsing_context_parent_top(scope, handle, parent, top);
        let global_template = self.bridge.bindings.window_global_template(scope);
        let indexed_db_manager = self.indexed_db_manager.clone();
        let storage_bucket_store = Some(self.storage_bucket_store.clone());
        let (context, runtime_observable_context_token, bridge_ref) =
            crate::script_vm::bootstrap_child_default_context_in_scope(
                scope,
                global_template,
                host,
                config.resource_owner_id,
                &config.promise_reject_dispatch,
                indexed_db_manager,
                storage_bucket_store,
                handle,
                owner,
            )?;
        let local_context = v8::Local::new(scope, &context);
        self.register_window_execution_context(WindowExecutionContextBinding::new(
            execution_context_owner,
            dispatch_scope,
            runtime_observable_context_token,
            v8::Global::new(scope, local_context),
        ));
        let child_scope = &mut v8::ContextScope::new(scope, local_context);
        self.bind_child_window_indexed_db_factory_after_context_registration(child_scope, handle);
        self.frame_owner_store
            .ensure_child_realm(handle)
            .ok_or_else(|| anyhow::anyhow!("missing child realm owner record"))?;
        pending_contexts.borrow_mut().insert(
            handle,
            PrebootstrappedChildDefaultContext {
                local_window_id: owner.local_window_id,
                context,
                bridge_ref,
                runtime_observable_context_token,
            },
        );
        self.request_child_frame_realm_materialization(handle);
        Ok(local_context)
    }

    pub(crate) fn configure_child_isolated_world_global<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
        handle: DomHandle,
        expected_owner: FrameDocumentTaskOwner,
        realm_token: RuntimeObservableContextToken,
        access_policy: WindowExecutionContextAccessPolicy,
    ) -> Result<()> {
        let browsing_context_id = self
            .frame_owner_store
            .browsing_context_id_for_child_handle(handle)
            .ok_or_else(|| anyhow::anyhow!("missing child browsing-context identity"))?;
        self.configure_child_window_realm_global(
            scope,
            global,
            ChildWindowRealmInit {
                handle,
                projection: crate::browsing_context_model::RealmHostProjection::new(
                    browsing_context_id,
                    expected_owner,
                    realm_token,
                    WindowWorldKind::Isolated { access_policy },
                ),
            },
        )
    }

    pub(crate) fn configure_child_default_world_global<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
        handle: DomHandle,
        expected_owner: FrameDocumentTaskOwner,
        realm_token: RuntimeObservableContextToken,
    ) -> Result<()> {
        let browsing_context_id = self
            .frame_owner_store
            .browsing_context_id_for_child_handle(handle)
            .ok_or_else(|| anyhow::anyhow!("missing child browsing-context identity"))?;
        self.configure_child_window_realm_global(
            scope,
            global,
            ChildWindowRealmInit {
                handle,
                projection: crate::browsing_context_model::RealmHostProjection::new(
                    browsing_context_id,
                    expected_owner,
                    realm_token,
                    WindowWorldKind::Default,
                ),
            },
        )
    }

    fn configure_child_window_realm_global<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
        init: ChildWindowRealmInit,
    ) -> Result<()> {
        let projection = initialize_child_window_realm_state(self, scope, global, init)?;
        if init.projection.world().is_default() {
            self.child_window_proxy_records
                .set_realm_top(scope, init.handle, projection.top);
            self.install_default_world_state_for_child_window(
                scope,
                init.handle,
                global,
                projection.document,
            );
            self.install_child_window_proxy_cross_origin_access_surface(
                scope,
                init.handle,
                global,
                projection.parent,
                projection.top,
            );
        }
        Ok(())
    }
}
