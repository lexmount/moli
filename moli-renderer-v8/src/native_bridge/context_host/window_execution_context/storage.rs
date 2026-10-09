use std::{collections::HashMap, sync::Arc};

use crate::{
    RendererWebStorageHandles,
    context_bootstrap::WebStorageEventRoute,
    context_bootstrap::{SharedWebStorageStore, WebStorageArea, WebStorageAreaKind},
    native_bridge::{WindowExecutionContextOwner, WindowTaskTarget},
    page_task_queue::RendererPageStorageEventDeliverySender,
};

struct LocalWindowStorage {
    target: WindowTaskTarget,
    area_key: String,
    session_override: Option<SharedWebStorageStore>,
    local: WebStorageArea,
    session: WebStorageArea,
}

/// Storage services follow the LocalWindow, independently of its Document or
/// default/isolated realm. Every attached Window subscribes, including Windows
/// that only listen for storage events and never access a Storage getter.
#[derive(Default)]
pub(in crate::native_bridge::context_host) struct WindowStorageBindings {
    stores: RendererWebStorageHandles,
    route: Arc<WebStorageEventRoute>,
    windows: HashMap<WindowExecutionContextOwner, LocalWindowStorage>,
}

impl WindowStorageBindings {
    pub(in crate::native_bridge::context_host) fn bind_task_sender(
        &self,
        sender: RendererPageStorageEventDeliverySender,
    ) {
        self.route.bind(sender);
    }

    pub(in crate::native_bridge::context_host) fn attach(
        &mut self,
        target: WindowTaskTarget,
        area_key: String,
        session_override: Option<SharedWebStorageStore>,
    ) {
        if self
            .windows
            .get(&target.owner())
            .is_some_and(|window| window.target == target && window.area_key == area_key)
        {
            return;
        }
        let local = WebStorageArea::new(
            self.stores.local_storage(),
            area_key.clone(),
            WebStorageAreaKind::Local,
            target,
            self.route.clone(),
        );
        let session = WebStorageArea::new(
            session_override
                .clone()
                .unwrap_or_else(|| self.stores.session_storage()),
            area_key.clone(),
            WebStorageAreaKind::Session,
            target,
            self.route.clone(),
        );
        self.windows.insert(
            target.owner(),
            LocalWindowStorage {
                target,
                area_key,
                session_override,
                local,
                session,
            },
        );
    }

    pub(in crate::native_bridge::context_host) fn replace_stores(
        &mut self,
        stores: &RendererWebStorageHandles,
    ) {
        if self.stores.shares_local_storage_with(stores)
            && self.stores.shares_session_storage_with(stores)
        {
            return;
        }
        let local_changed = !self.stores.shares_local_storage_with(stores);
        let session_changed = !self.stores.shares_session_storage_with(stores);
        self.stores = stores.clone();
        for window in self.windows.values_mut() {
            if local_changed {
                window.local = WebStorageArea::new(
                    stores.local_storage(),
                    window.area_key.clone(),
                    WebStorageAreaKind::Local,
                    window.target,
                    self.route.clone(),
                );
            }
            if session_changed && window.session_override.is_none() {
                window.session = WebStorageArea::new(
                    stores.session_storage(),
                    window.area_key.clone(),
                    WebStorageAreaKind::Session,
                    window.target,
                    self.route.clone(),
                );
            }
        }
    }

    pub(in crate::native_bridge::context_host) fn local_store(&self) -> SharedWebStorageStore {
        self.stores.local_storage()
    }
    pub(in crate::native_bridge::context_host) fn session_store(&self) -> SharedWebStorageStore {
        self.stores.session_storage()
    }

    pub(in crate::native_bridge::context_host) fn area(
        &self,
        target: WindowTaskTarget,
        is_session: bool,
    ) -> Option<WebStorageArea> {
        let window = self
            .windows
            .get(&target.owner())
            .filter(|window| window.target == target)?;
        Some(if is_session {
            window.session.clone()
        } else {
            window.local.clone()
        })
    }

    pub(in crate::native_bridge::context_host) fn retire(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) {
        self.windows.remove(&owner);
    }

    pub(in crate::native_bridge::context_host) fn close(&mut self) {
        self.windows.clear();
        self.route.close();
    }
}
