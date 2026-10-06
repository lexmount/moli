use super::*;
use crate::native_bridge::context_host::{
    JsContextHostBridgeRef, RuntimeObservableContextToken, WindowExecutionContextBinding,
    WindowExecutionContextOwner,
};
use std::rc::Weak;

pub(crate) struct PopupDefaultContext {
    local_window_id: LightweightPopupLocalWindowId,
    pub(crate) context: v8::Global<v8::Context>,
    // Owned by ScriptVm, independently of the host this token retains.
    _bridge_ref: JsContextHostBridgeRef,
    pub(crate) realm_token: RuntimeObservableContextToken,
}

pub(crate) type SharedPopupDefaultContexts = Rc<RefCell<HashMap<u64, PopupDefaultContext>>>;
pub(in crate::native_bridge::context_host) type WeakPopupDefaultContexts =
    Weak<RefCell<HashMap<u64, PopupDefaultContext>>>;

impl JsContextHost {
    pub(crate) fn popup_secure_context_available(&self, popup_id: u64) -> bool {
        self.lightweight_popup_document_record(popup_id)
            .is_some_and(|document| document.state.secure_context_available)
    }

    pub(crate) fn take_popup_window_proxy_for_realm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        popup_id: u64,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let record = self
            .lightweight_popup_browsing_contexts
            .get_mut(&popup_id)?;
        if let Some(context) = record.facade_context.take() {
            v8::Local::new(scope, &context).detach_global();
        }
        Some(v8::Local::new(scope, &record.window_proxy))
    }

    pub(crate) fn bind_popup_window_context_owner_before_runtime_bootstrap(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        window: v8::Local<'_, v8::Object>,
        popup_id: u64,
    ) -> Result<()> {
        anyhow::ensure!(
            self.lightweight_popup_is_open(popup_id),
            "missing popup LocalWindow"
        );
        set_private_value(
            scope,
            window,
            LIGHTWEIGHT_POPUP_ID_SLOT,
            v8::BigInt::new_from_u64(scope, popup_id).into(),
        );
        Ok(())
    }

    pub(crate) fn ensure_popup_default_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
    ) -> Result<v8::Local<'s, v8::Context>> {
        let local_window_id = self
            .lightweight_popup_document_record(popup_id)
            .ok_or_else(|| anyhow::anyhow!("missing popup Document"))?
            .local_window_id;
        let owner = WindowExecutionContextOwner::LightweightPopup {
            popup_id,
            local_window_id,
        };
        if let Some((_, context)) = self.window_execution_context(
            scope,
            owner,
            OwnerDispatchScope::LightweightPopup(popup_id),
        ) {
            return Ok(context);
        }
        let config = self
            .window_default_context_bootstrap
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Window realm bootstrap is unavailable"))?;
        let contexts = config
            .popup_contexts
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("popup realm owner was retired"))?;
        if let Some(context) = contexts.borrow().get(&popup_id)
            && context.local_window_id == local_window_id
        {
            return Ok(v8::Local::new(scope, &context.context));
        }
        let retired = contexts.borrow_mut().remove(&popup_id);
        if let Some(retired) = retired {
            let context = v8::Local::new(scope, &retired.context);
            {
                let scope = &mut v8::ContextScope::new(scope, context);
                crate::native_bridge::clear_context_wrapper_cache_for_teardown(scope, false);
            }
            self.retire_window_execution_contexts_for_context_token(
                retired.realm_token,
                Some(config.resource_owner_id),
            );
            context.detach_global();
        }
        let host = config
            .host
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("popup context host was retired"))?;
        let global_template = self.bridge.bindings.window_global_template(scope);
        let (context, realm_token, bridge_ref) =
            crate::script_vm::bootstrap_popup_default_context_in_scope(
                scope,
                global_template,
                host,
                config.resource_owner_id,
                &config.promise_reject_dispatch,
                self.indexed_db_manager.clone(),
                Some(self.storage_bucket_store.clone()),
                popup_id,
                local_window_id,
            )?;
        let local_context = v8::Local::new(scope, &context);
        self.register_window_execution_context(WindowExecutionContextBinding::new(
            WindowExecutionContextOwner::LightweightPopup {
                popup_id,
                local_window_id,
            },
            OwnerDispatchScope::LightweightPopup(popup_id),
            realm_token,
            v8::Global::new(scope, local_context),
        ));
        contexts.borrow_mut().insert(
            popup_id,
            PopupDefaultContext {
                local_window_id,
                context,
                _bridge_ref: bridge_ref,
                realm_token,
            },
        );
        Ok(local_context)
    }

    pub(crate) fn configure_popup_default_world_global<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        window: v8::Local<'s, v8::Object>,
        popup_id: u64,
    ) -> Result<()> {
        let document = self
            .lightweight_popup_document_record(popup_id)
            .ok_or_else(|| anyhow::anyhow!("missing popup Document during bootstrap"))?;
        let url = document.url.clone();
        let local_window_id = document.local_window_id;
        let secure = document.state.secure_context_available;
        let name = self
            .lightweight_popup_browsing_contexts
            .get(&popup_id)
            .ok_or_else(|| anyhow::anyhow!("missing popup browsing context"))?
            .name
            .get();
        let name = v8_string(scope, &name)
            .ok_or_else(|| anyhow::anyhow!("popup name allocation failed"))?;
        set_private_value(scope, window, WINDOW_NAME_SLOT, name.into());
        LightweightPopupWindowNameDeclaration::default().initialize(scope, window)?;
        LightweightPopupWindowOriginDeclaration::default().initialize(scope, window)?;
        LightweightPopupWindowOpenerDeclaration {
            popup_id: v8::BigInt::new_from_u64(scope, popup_id),
            opener: (),
        }
        .initialize(scope, window)?;
        install_simple_event_target_methods(
            scope,
            window,
            LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT,
            false,
        );
        install_lightweight_popup_event_handler_accessors(scope, window, secure);
        LightweightPopupWindowMethodsDeclaration::default().initialize(scope, window)?;
        install_storage_aliases_for_window(scope, window)?;
        crate::context_bootstrap::reset_window_location_runtime_state(scope, window, url.as_str())?;
        sync_window_location_history_navigation_runtime_surface(scope, window);
        crate::context_bootstrap::install_window_bar_props(
            scope,
            window,
            Some((popup_id, local_window_id)),
        )?;
        self.refresh_lightweight_popup_navigator(scope, popup_id, window);
        self.refresh_lightweight_popup_indexed_db_factory(scope, popup_id, window);

        super::super::child_frame_runtime::install_popup_window_cross_origin_access_surface(
            scope, window, popup_id,
        );
        Ok(())
    }
}
