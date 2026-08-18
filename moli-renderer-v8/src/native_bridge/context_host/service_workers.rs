use super::{JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WorkerOwnerScope};
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::runtime::ServiceWorkerControlState;
use crate::service_worker_runtime::ServiceWorkerNotificationAction;
use crate::service_worker_runtime::{
    ServiceWorkerClientFrameType, ServiceWorkerClientId, ServiceWorkerClientSnapshot,
    ServiceWorkerClientsOpenWindowError, ServiceWorkerFetchDispatch, ServiceWorkerFetchRequest,
    ServiceWorkerFetchRequestMetadata, ServiceWorkerNavigationPreloadState,
    ServiceWorkerNavigationPreloadStateError, ServiceWorkerPushSubscriptionSnapshot,
    ServiceWorkerRegistrationSnapshot, ServiceWorkerRequestDestination,
    ServiceWorkerUnregisterStart, ServiceWorkerUpdateViaCache, ServiceWorkerVersionId,
};
use crate::structured_clone::V8StructuredClonePayload;
use crate::worker::{WorkerNetworkPolicy, WorkerScriptKind, worker_secure_context_for_script_url};
use moli_storage_key::MoliStorageKey;
use url::Url;

pub(super) fn service_worker_first_party_storage_key(document_url: &Url) -> String {
    moli_storage_key::MoliStorageKey::first_party_from_url(document_url, None)
        .serialized_storage_key()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerWindowOwner {
    dispatch_scope: OwnerDispatchScope,
    document_owner: WindowDocumentOwner,
}

impl ServiceWorkerWindowOwner {
    pub(crate) fn dispatch_scope(self) -> OwnerDispatchScope {
        self.dispatch_scope
    }

    pub(crate) fn document_owner(self) -> Option<FrameDocumentTaskOwner> {
        self.document_owner.frame_document_owner()
    }

    pub(crate) fn window_document_owner(self) -> WindowDocumentOwner {
        self.document_owner
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerWindowRequestContext {
    owner: ServiceWorkerWindowOwner,
    document_url: Url,
    storage_key: MoliStorageKey,
}

impl ServiceWorkerWindowRequestContext {
    pub(crate) fn owner(&self) -> ServiceWorkerWindowOwner {
        self.owner
    }

    pub(crate) fn document_url(&self) -> &Url {
        &self.document_url
    }

    fn serialized_storage_key(&self) -> String {
        self.storage_key.serialized_storage_key()
    }

    fn storage_key_top_level_site(&self) -> String {
        self.storage_key.top_level_site().to_owned()
    }
}

pub(crate) struct PendingServiceWorkerRegister {
    pub(crate) owner: ServiceWorkerWindowOwner,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) resolver: v8::Global<v8::PromiseResolver>,
}

pub(crate) struct PendingServiceWorkerReady {
    pub(crate) request_id: u64,
    pub(crate) request_context: ServiceWorkerWindowRequestContext,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) resolver: v8::Global<v8::PromiseResolver>,
    pub(crate) attached: bool,
}

pub(crate) struct PendingServiceWorkerUnregister {
    pub(crate) owner: ServiceWorkerWindowOwner,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) resolver: v8::Global<v8::PromiseResolver>,
    pub(crate) registration: v8::Global<v8::Object>,
    pub(crate) active_worker: Option<v8::Global<v8::Object>>,
}

pub(crate) struct ServiceWorkerRegistrationWatcher {
    pub(crate) owner: ServiceWorkerWindowOwner,
    pub(crate) scope_url: Url,
    pub(crate) storage_key: String,
    pub(crate) registration: v8::Weak<v8::Object>,
}

#[derive(Clone)]
pub(crate) struct PendingServiceWorkerClientsOpenWindowPopup {
    request_id: u64,
    source_version_id: ServiceWorkerVersionId,
    source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    document_owner: crate::native_bridge::LightweightPopupDocumentOwner,
}

impl JsContextHost {
    fn service_worker_task_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageServiceWorkerTaskSender {
        self.service_worker_task_tx.clone()
    }

    fn service_worker_window_owner_for_dispatch_scope(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<ServiceWorkerWindowOwner> {
        let document_owner = match dispatch_scope {
            OwnerDispatchScope::Top => {
                WindowDocumentOwner::Frame(self.current_main_document_task_owner()?)
            }
            OwnerDispatchScope::Child(handle) => WindowDocumentOwner::Frame(
                self.frame_owner_store
                    .current_child_document_task_owner(handle)?,
            ),
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                WindowDocumentOwner::LightweightPopup(
                    self.current_lightweight_popup_document_owner(popup_id)?,
                )
            }
        };
        Some(ServiceWorkerWindowOwner {
            dispatch_scope,
            document_owner,
        })
    }

    #[cfg(test)]
    pub(crate) fn service_worker_window_client_target_for_test(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<crate::types::ServiceWorkerWindowClientTarget> {
        let owner = self.service_worker_window_owner_for_dispatch_scope(dispatch_scope)?;
        Some(crate::types::ServiceWorkerWindowClientTarget {
            client_id: self.service_worker_client_id_for_subresource_owner(dispatch_scope),
            document_owner: owner.window_document_owner(),
        })
    }

    pub(crate) fn service_worker_window_request_context(
        &mut self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<ServiceWorkerWindowRequestContext> {
        let owner = self.service_worker_window_owner_for_dispatch_scope(dispatch_scope)?;
        let (document_url, storage_key) = match dispatch_scope {
            OwnerDispatchScope::Top => (
                self.document_url().clone(),
                self.top_document_storage_context().storage_key().clone(),
            ),
            OwnerDispatchScope::Child(handle) => (
                self.child_browsing_context_current_url(handle)?,
                self.storage_context_for_child_browsing_context(handle)?
                    .storage_key()
                    .clone(),
            ),
            OwnerDispatchScope::LightweightPopup(popup_id) => (
                self.lightweight_popup_document_url(popup_id)?,
                self.storage_context_for_lightweight_popup(popup_id)?
                    .storage_key()
                    .clone(),
            ),
        };
        Some(ServiceWorkerWindowRequestContext {
            owner,
            document_url,
            storage_key,
        })
    }

    pub(crate) fn service_worker_window_owner_is_current(
        &self,
        owner: ServiceWorkerWindowOwner,
    ) -> bool {
        self.window_document_owner_is_current(owner.window_document_owner())
            && match (owner.dispatch_scope(), owner.window_document_owner()) {
                (OwnerDispatchScope::Top, WindowDocumentOwner::Frame(document_owner)) => {
                    self.current_main_document_task_owner() == Some(document_owner)
                }
                (OwnerDispatchScope::Child(handle), WindowDocumentOwner::Frame(document_owner)) => {
                    self.frame_owner_store
                        .current_child_document_task_owner(handle)
                        == Some(document_owner)
                }
                (
                    OwnerDispatchScope::LightweightPopup(popup_id),
                    WindowDocumentOwner::LightweightPopup(document_owner),
                ) => document_owner.popup_id() == popup_id,
                _ => false,
            }
    }

    pub(crate) fn window_document_owner_is_current(&self, owner: WindowDocumentOwner) -> bool {
        match owner {
            WindowDocumentOwner::Frame(owner) => {
                self.frame_owner_store.document_task_owner_is_current(owner)
            }
            WindowDocumentOwner::LightweightPopup(owner) => {
                self.lightweight_popup_document_owner_is_current(owner)
            }
        }
    }

    pub(crate) fn window_document_owner_is_current_for_dispatch_scope(
        &self,
        owner: WindowDocumentOwner,
        dispatch_scope: OwnerDispatchScope,
    ) -> bool {
        match (owner, dispatch_scope) {
            (WindowDocumentOwner::Frame(owner), OwnerDispatchScope::Top) => {
                self.current_main_document_task_owner() == Some(owner)
            }
            (WindowDocumentOwner::Frame(owner), OwnerDispatchScope::Child(handle)) => {
                self.current_child_document_task_owner(handle) == Some(owner)
            }
            (
                WindowDocumentOwner::LightweightPopup(owner),
                OwnerDispatchScope::LightweightPopup(popup_id),
            ) => {
                owner.popup_id() == popup_id
                    && self.current_lightweight_popup_document_owner(popup_id) == Some(owner)
            }
            _ => false,
        }
    }

    pub(crate) fn retire_service_worker_document_owner(
        &mut self,
        retired_owner: FrameDocumentTaskOwner,
    ) {
        self.retire_service_worker_window_document_owner(WindowDocumentOwner::Frame(retired_owner));
    }

    pub(crate) fn retire_service_worker_window_document_owner(
        &mut self,
        retired_owner: WindowDocumentOwner,
    ) {
        let register_count = self.pending_service_worker_registers.len();
        self.pending_service_worker_registers
            .retain(|_, pending| pending.owner.window_document_owner() != retired_owner);
        let unregister_count = self.pending_service_worker_unregisters.len();
        self.pending_service_worker_unregisters
            .retain(|_, pending| pending.owner.window_document_owner() != retired_owner);
        let ready_count = self.pending_service_worker_ready.len();
        self.pending_service_worker_ready.retain(|_, pending| {
            pending.request_context.owner().window_document_owner() != retired_owner
        });
        let watcher_count = self.service_worker_registration_watchers.len();
        self.service_worker_registration_watchers
            .retain(|watcher| watcher.owner.window_document_owner() != retired_owner);
        tracing::debug!(
            ?retired_owner,
            retired_register_resolvers =
                register_count - self.pending_service_worker_registers.len(),
            retired_unregister_resolvers =
                unregister_count - self.pending_service_worker_unregisters.len(),
            retired_ready_resolvers = ready_count - self.pending_service_worker_ready.len(),
            retired_lifecycle_watchers =
                watcher_count - self.service_worker_registration_watchers.len(),
            "retired service worker window state with document owner"
        );
    }

    pub(crate) fn apply_main_service_worker_document_owner_transition(
        &mut self,
        transition: crate::frame_owner_model::MainDocumentOwnerTransition,
    ) {
        let retired_owner = transition.retired_owner();
        let register_count = self.pending_service_worker_registers.len();
        self.pending_service_worker_registers
            .retain(|_, pending| pending.owner.document_owner() != Some(retired_owner));
        let unregister_count = self.pending_service_worker_unregisters.len();
        self.pending_service_worker_unregisters
            .retain(|_, pending| pending.owner.document_owner() != Some(retired_owner));
        let ready_count = self.pending_service_worker_ready.len();
        self.pending_service_worker_ready.retain(|_, pending| {
            pending.request_context.owner().document_owner() != Some(retired_owner)
        });

        let current_window_owner = ServiceWorkerWindowOwner {
            dispatch_scope: OwnerDispatchScope::Top,
            document_owner: WindowDocumentOwner::Frame(transition.current_owner()),
        };
        let mut rebound_scopes = Vec::new();
        self.service_worker_registration_watchers
            .retain(|watcher| !watcher.registration.is_empty());
        for watcher in &mut self.service_worker_registration_watchers {
            if watcher.owner.document_owner() != Some(retired_owner) {
                continue;
            }
            watcher.owner = current_window_owner;
            rebound_scopes.push((watcher.scope_url.clone(), watcher.storage_key.clone()));
        }
        self.service_worker_lifecycle_watched_scopes
            .retain(|(_, _, owner)| *owner != WindowDocumentOwner::Frame(retired_owner));
        let mut rebound_watcher_count = 0usize;
        for (scope_url, storage_key) in rebound_scopes {
            rebound_watcher_count += 1;
            if self.service_worker_lifecycle_watched_scopes.insert((
                scope_url.clone(),
                storage_key.clone(),
                current_window_owner.window_document_owner(),
            )) {
                self.browser_context_runtime
                    .watch_service_worker_registration_lifecycle(
                        scope_url,
                        storage_key,
                        current_window_owner.window_document_owner(),
                        self.service_worker_task_sender(),
                    );
            }
        }
        tracing::debug!(
            ?retired_owner,
            current_owner = ?transition.current_owner(),
            retired_register_resolvers =
                register_count - self.pending_service_worker_registers.len(),
            retired_unregister_resolvers =
                unregister_count - self.pending_service_worker_unregisters.len(),
            retired_ready_resolvers = ready_count - self.pending_service_worker_ready.len(),
            rebound_lifecycle_watchers = rebound_watcher_count,
            "applied main service worker document owner transition"
        );
    }

    pub(crate) fn update_main_service_worker_client_after_document_replacement(
        &mut self,
        transition: crate::frame_owner_model::MainDocumentOwnerTransition,
    ) {
        let document_url = self.document_url().clone();
        let storage_key = self.service_worker_document_storage_key();
        let document_owner = WindowDocumentOwner::Frame(transition.current_owner());
        let updated = self
            .browser_context_runtime
            .update_service_worker_client_document(
                self.service_worker_client_id,
                document_url.clone(),
                storage_key,
                ServiceWorkerClientFrameType::TopLevel,
                Some(document_owner),
            );
        if updated {
            tracing::debug!(
                retired_owner = ?transition.retired_owner(),
                current_owner = ?transition.current_owner(),
                client_id = ?self.service_worker_client_id,
                ?document_owner,
                document_url = %document_url,
                "updated top-level service worker client in main document owner transaction"
            );
        } else {
            tracing::warn!(
                retired_owner = ?transition.retired_owner(),
                current_owner = ?transition.current_owner(),
                client_id = ?self.service_worker_client_id,
                ?document_owner,
                document_url = %document_url,
                "main document owner transaction could not update its service worker client"
            );
        }
    }

    fn service_worker_document_storage_key(&mut self) -> String {
        self.top_document_storage_context()
            .storage_key()
            .serialized_storage_key()
    }

    pub(crate) fn register_pending_service_worker_register(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        owner: ServiceWorkerWindowOwner,
    ) -> (
        u64,
        WindowDocumentOwner,
        crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) {
        let request_id = self.next_service_worker_register_request_id;
        self.next_service_worker_register_request_id = self
            .next_service_worker_register_request_id
            .wrapping_add(1)
            .max(1);
        self.pending_service_worker_registers.insert(
            request_id,
            PendingServiceWorkerRegister {
                owner,
                context: v8::Global::new(scope, scope.get_current_context()),
                resolver: v8::Global::new(scope, resolver),
            },
        );
        tracing::debug!(
            request_id,
            dispatch_scope = ?owner.dispatch_scope(),
            window_document_owner = ?owner.window_document_owner(),
            "registered owner-bound service worker register resolver"
        );
        (
            request_id,
            owner.window_document_owner(),
            self.service_worker_task_sender(),
        )
    }

    pub(crate) fn take_pending_service_worker_register(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerRegister> {
        self.pending_service_worker_registers.remove(&request_id)
    }

    #[cfg(test)]
    pub(crate) fn pending_service_worker_register_owners_for_test(
        &self,
    ) -> Vec<(u64, ServiceWorkerWindowOwner)> {
        let mut owners = self
            .pending_service_worker_registers
            .iter()
            .map(|(request_id, pending)| (*request_id, pending.owner))
            .collect::<Vec<_>>();
        owners.sort_by_key(|(request_id, _)| *request_id);
        owners
    }

    pub(crate) fn register_pending_service_worker_unregister(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        registration: v8::Local<'_, v8::Object>,
        active_worker: Option<v8::Local<'_, v8::Object>>,
        owner: ServiceWorkerWindowOwner,
    ) -> (
        u64,
        WindowDocumentOwner,
        crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) {
        let request_id = self.next_service_worker_register_request_id;
        self.next_service_worker_register_request_id = self
            .next_service_worker_register_request_id
            .wrapping_add(1)
            .max(1);
        self.pending_service_worker_unregisters.insert(
            request_id,
            PendingServiceWorkerUnregister {
                owner,
                context: v8::Global::new(scope, scope.get_current_context()),
                resolver: v8::Global::new(scope, resolver),
                registration: v8::Global::new(scope, registration),
                active_worker: active_worker.map(|worker| v8::Global::new(scope, worker)),
            },
        );
        tracing::debug!(
            request_id,
            dispatch_scope = ?owner.dispatch_scope(),
            window_document_owner = ?owner.window_document_owner(),
            "registered owner-bound service worker unregister resolver"
        );
        (
            request_id,
            owner.window_document_owner(),
            self.service_worker_task_sender(),
        )
    }

    pub(crate) fn take_pending_service_worker_unregister(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerUnregister> {
        self.pending_service_worker_unregisters.remove(&request_id)
    }

    pub(crate) fn install_pending_service_worker_ready(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        request_context: ServiceWorkerWindowRequestContext,
    ) -> u64 {
        let request_id = self.next_service_worker_register_request_id;
        self.next_service_worker_register_request_id = self
            .next_service_worker_register_request_id
            .wrapping_add(1)
            .max(1);
        let owner = request_context.owner();
        self.pending_service_worker_ready.insert(
            request_id,
            PendingServiceWorkerReady {
                request_id,
                request_context,
                context: v8::Global::new(scope, scope.get_current_context()),
                resolver: v8::Global::new(scope, resolver),
                attached: false,
            },
        );
        tracing::debug!(
            request_id,
            dispatch_scope = ?owner.dispatch_scope(),
            window_document_owner = ?owner.window_document_owner(),
            "registered owner-bound service worker ready resolver"
        );
        request_id
    }

    fn pending_service_worker_ready_requests(
        &self,
    ) -> Vec<(
        u64,
        ServiceWorkerWindowRequestContext,
        crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    )> {
        self.pending_service_worker_ready
            .values()
            .filter(|pending| !pending.attached)
            .map(|pending| {
                (
                    pending.request_id,
                    pending.request_context.clone(),
                    self.service_worker_task_sender(),
                )
            })
            .collect()
    }

    pub(crate) fn take_pending_service_worker_ready(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerReady> {
        self.pending_service_worker_ready.remove(&request_id)
    }

    #[cfg(test)]
    pub(crate) fn pending_service_worker_ready_owners_for_test(
        &self,
    ) -> Vec<(u64, ServiceWorkerWindowOwner)> {
        let mut owners = self
            .pending_service_worker_ready
            .iter()
            .map(|(request_id, pending)| (*request_id, pending.request_context.owner()))
            .collect::<Vec<_>>();
        owners.sort_by_key(|(request_id, _)| *request_id);
        owners
    }

    #[cfg(test)]
    pub(crate) fn service_worker_registration_watchers_for_test(
        &self,
    ) -> Vec<(ServiceWorkerWindowOwner, Url, String)> {
        self.service_worker_registration_watchers
            .iter()
            .map(|watcher| {
                (
                    watcher.owner,
                    watcher.scope_url.clone(),
                    watcher.storage_key.clone(),
                )
            })
            .collect()
    }

    pub(crate) fn start_service_worker_runtime(
        &mut self,
        script_url: Url,
        scope_url: Url,
        script_kind: WorkerScriptKind,
        update_via_cache: ServiceWorkerUpdateViaCache,
        request_context: &ServiceWorkerWindowRequestContext,
        request_client: crate::network::ResourceRequestClient,
        register_request_id: u64,
        register_document_owner: WindowDocumentOwner,
        register_completion_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) {
        let creator_secure_context =
            moli_url::is_potentially_trustworthy_url(request_context.document_url());
        let network_policy = WorkerNetworkPolicy {
            secure_context: worker_secure_context_for_script_url(
                &script_url,
                creator_secure_context,
            ),
            permission_overrides: self.permission_overrides().to_vec(),
            extra_http_headers: self.extra_http_headers().to_vec(),
            network_offline: self.network_offline(),
            blocked_url_patterns: self.blocked_url_patterns().to_vec(),
            network_partition_key: None,
            fetch_subresource_interception_enabled: self.fetch_subresource_interception_enabled(),
            fetch_subresource_interception_resource_type: self
                .fetch_subresource_interception_resource_type(),
        };
        let document_url = request_context.document_url().clone();
        let storage_key = request_context.serialized_storage_key();
        let storage_key_top_level_site = Some(request_context.storage_key_top_level_site());
        self.browser_context_runtime
            .service_worker_runtime()
            .start_registration_with_storage_key(
                script_url,
                scope_url,
                document_url,
                storage_key,
                script_kind,
                request_client,
                network_policy,
                self.browser_context_runtime(),
                storage_key_top_level_site,
                self.indexed_db_manager(),
                Some(self.storage_bucket_store()),
                update_via_cache,
                register_request_id,
                register_document_owner,
                register_completion_tx,
            );
        self.watch_pending_service_worker_ready();
    }

    pub(crate) fn watch_pending_service_worker_ready(&mut self) -> bool {
        let requests = self.pending_service_worker_ready_requests();
        let mut attached_any = false;
        for (request_id, request_context, completion_tx) in requests {
            if self
                .browser_context_runtime
                .watch_service_worker_ready_registration(
                    request_context.document_url().clone(),
                    request_context.serialized_storage_key(),
                    request_id,
                    request_context.owner().window_document_owner(),
                    completion_tx,
                )
            {
                if let Some(pending) = self.pending_service_worker_ready.get_mut(&request_id) {
                    pending.attached = true;
                }
                attached_any = true;
            }
        }
        attached_any
    }

    pub(crate) fn show_service_worker_notification(
        &self,
        scope_url: &Url,
        title: String,
        tag: String,
        metadata: crate::runtime::ServiceWorkerNotificationMetadata,
        actions: Vec<ServiceWorkerNotificationAction>,
        data: V8StructuredClonePayload,
    ) -> bool {
        self.browser_context_runtime
            .show_service_worker_notification(scope_url, title, tag, metadata, actions, data)
    }

    pub(crate) fn service_worker_notifications(
        &self,
        scope_url: &Url,
        tag: Option<&str>,
    ) -> Vec<crate::runtime::ServiceWorkerNotificationSnapshot> {
        self.browser_context_runtime
            .service_worker_notifications(scope_url, tag)
    }

    pub(crate) fn register_service_worker_sync(&self, scope_url: &Url, tag: String) -> bool {
        self.browser_context_runtime
            .register_service_worker_sync(scope_url, tag)
    }

    pub(crate) fn service_worker_sync_tags(&self, scope_url: &Url) -> Vec<String> {
        self.browser_context_runtime
            .service_worker_sync_tags(scope_url)
    }

    pub(crate) fn register_service_worker_periodic_sync(
        &self,
        scope_url: &Url,
        tag: String,
        min_interval_ms: u64,
    ) -> bool {
        self.browser_context_runtime
            .register_service_worker_periodic_sync(scope_url, tag, min_interval_ms)
    }

    pub(crate) fn service_worker_periodic_sync_tags(&self, scope_url: &Url) -> Vec<String> {
        self.browser_context_runtime
            .service_worker_periodic_sync_tags(scope_url)
    }

    pub(crate) fn unregister_service_worker_periodic_sync(
        &self,
        scope_url: &Url,
        tag: &str,
    ) -> bool {
        self.browser_context_runtime
            .unregister_service_worker_periodic_sync(scope_url, tag)
    }

    pub(crate) fn service_worker_navigation_preload_state(
        &self,
        scope_url: &Url,
    ) -> Option<ServiceWorkerNavigationPreloadState> {
        self.browser_context_runtime
            .service_worker_navigation_preload_state(scope_url)
    }

    pub(crate) fn set_service_worker_navigation_preload_enabled(
        &self,
        scope_url: &Url,
        enabled: bool,
    ) -> Result<(), ServiceWorkerNavigationPreloadStateError> {
        self.browser_context_runtime
            .set_service_worker_navigation_preload_enabled(scope_url, enabled)
    }

    pub(crate) fn set_service_worker_navigation_preload_header_value(
        &self,
        scope_url: &Url,
        header_value: String,
    ) -> Result<(), ServiceWorkerNavigationPreloadStateError> {
        self.browser_context_runtime
            .set_service_worker_navigation_preload_header_value(scope_url, header_value)
    }

    pub(crate) fn subscribe_service_worker_push(
        &self,
        scope_url: &Url,
        user_visible_only: bool,
    ) -> Option<ServiceWorkerPushSubscriptionSnapshot> {
        self.browser_context_runtime
            .subscribe_service_worker_push(scope_url, user_visible_only)
    }

    pub(crate) fn service_worker_push_subscription(
        &self,
        scope_url: &Url,
    ) -> Option<ServiceWorkerPushSubscriptionSnapshot> {
        self.browser_context_runtime
            .service_worker_push_subscription(scope_url)
    }

    pub(crate) fn unsubscribe_service_worker_push(&self, scope_url: &Url) -> bool {
        self.browser_context_runtime
            .unsubscribe_service_worker_push(scope_url)
    }

    pub(crate) fn close_service_worker_notification(
        &self,
        registration_id: crate::runtime::ServiceWorkerRegistrationId,
        notification_id: u64,
    ) -> bool {
        self.browser_context_runtime
            .close_service_worker_notification(registration_id, notification_id)
    }

    pub(crate) fn watch_service_worker_registration_lifecycle(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        dispatch_scope: OwnerDispatchScope,
        scope_url: Url,
        registration: v8::Local<'_, v8::Object>,
    ) {
        let Some(request_context) = self.service_worker_window_request_context(dispatch_scope)
        else {
            tracing::debug!(
                ?dispatch_scope,
                %scope_url,
                "skipped service worker lifecycle watcher for stale window owner"
            );
            return;
        };
        let owner = request_context.owner();
        let storage_key = request_context.serialized_storage_key();
        self.compact_service_worker_registration_watchers();
        if self
            .service_worker_registration_watchers
            .iter()
            .any(|watcher| {
                watcher.owner == owner
                    && watcher.scope_url == scope_url
                    && watcher.storage_key == storage_key
                    && !watcher.registration.is_empty()
            })
        {
            return;
        }
        self.service_worker_registration_watchers
            .push(ServiceWorkerRegistrationWatcher {
                owner,
                scope_url: scope_url.clone(),
                storage_key: storage_key.clone(),
                registration: v8::Weak::new(scope, registration),
            });
        if self.service_worker_lifecycle_watched_scopes.insert((
            scope_url.clone(),
            storage_key.clone(),
            owner.window_document_owner(),
        )) {
            self.browser_context_runtime
                .watch_service_worker_registration_lifecycle(
                    scope_url,
                    storage_key,
                    owner.window_document_owner(),
                    self.service_worker_task_sender(),
                );
        }
        tracing::debug!(
            ?dispatch_scope,
            window_document_owner = ?owner.window_document_owner(),
            "registered owner-bound service worker lifecycle watcher"
        );
    }

    pub(crate) fn service_worker_registration_watchers_for_lifecycle<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        scope_url: &Url,
        storage_key: &str,
        document_owner: WindowDocumentOwner,
    ) -> Vec<(OwnerDispatchScope, v8::Local<'s, v8::Object>)> {
        self.compact_service_worker_registration_watchers();
        self.service_worker_registration_watchers
            .iter()
            .filter(|watcher| {
                watcher.owner.window_document_owner() == document_owner
                    && watcher.scope_url == *scope_url
                    && watcher.storage_key == storage_key
            })
            .filter_map(|watcher| {
                watcher
                    .registration
                    .to_local(scope)
                    .map(|registration| (watcher.owner.dispatch_scope(), registration))
            })
            .collect()
    }

    fn compact_service_worker_registration_watchers(&mut self) {
        let mut watchers = std::mem::take(&mut self.service_worker_registration_watchers);
        watchers.retain(|watcher| {
            !watcher.registration.is_empty()
                && self.service_worker_window_owner_is_current(watcher.owner)
        });
        let current_owners = watchers
            .iter()
            .map(|watcher| watcher.owner.window_document_owner())
            .collect::<std::collections::HashSet<_>>();
        self.service_worker_registration_watchers = watchers;
        self.service_worker_lifecycle_watched_scopes
            .retain(|(_, _, owner)| current_owners.contains(owner));
    }

    pub(crate) fn unregister_service_worker_control(
        &mut self,
        request_context: &ServiceWorkerWindowRequestContext,
        request_id: u64,
        document_owner: WindowDocumentOwner,
        completion_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerUnregisterStart {
        let state = self.service_worker_control_state_for_window_owner(
            request_context.owner().dispatch_scope(),
        );
        if let Some(state) = state.as_ref() {
            return self.unregister_service_worker_scope(
                request_context,
                state.scope_url(),
                request_id,
                document_owner,
                completion_tx,
            );
        }
        ServiceWorkerUnregisterStart::Completed(false)
    }

    pub(crate) fn unregister_service_worker_scope(
        &mut self,
        request_context: &ServiceWorkerWindowRequestContext,
        scope_url: &Url,
        request_id: u64,
        document_owner: WindowDocumentOwner,
        completion_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerUnregisterStart {
        self.browser_context_runtime
            .unregister_service_worker_scope(
                scope_url,
                request_context.serialized_storage_key(),
                request_id,
                document_owner,
                completion_tx,
            )
    }

    pub(crate) fn service_worker_control_state(&self) -> Option<ServiceWorkerControlState> {
        self.browser_context_runtime
            .service_worker_controller_for_client(self.service_worker_client_id)
            .or_else(|| self.service_worker_control.clone())
    }

    pub(crate) fn service_worker_control_state_for_window_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<ServiceWorkerControlState> {
        match owner {
            OwnerDispatchScope::Top => self.service_worker_control_state(),
            OwnerDispatchScope::Child(handle) => self
                .child_browsing_contexts
                .get(&handle)
                .and_then(|entry| entry.service_worker_client_id())
                .and_then(|client_id| {
                    self.browser_context_runtime
                        .service_worker_controller_for_client(client_id)
                }),
            OwnerDispatchScope::LightweightPopup(popup_id) => self
                .service_worker_popup_clients
                .get(&popup_id)
                .copied()
                .and_then(|client_id| {
                    self.browser_context_runtime
                        .service_worker_controller_for_client(client_id)
                }),
        }
    }

    pub(crate) fn service_worker_client_id(&self) -> ServiceWorkerClientId {
        self.service_worker_client_id
    }

    fn service_worker_child_client_handle(
        &self,
        client_id: ServiceWorkerClientId,
    ) -> Option<crate::document_runtime::DomHandle> {
        self.child_browsing_contexts
            .iter()
            .find_map(|(handle, entry)| {
                entry
                    .has_service_worker_client_id(client_id)
                    .then_some(*handle)
            })
    }

    fn service_worker_window_client_owner(
        &self,
        client_id: ServiceWorkerClientId,
    ) -> Option<OwnerDispatchScope> {
        if client_id == self.service_worker_client_id {
            return Some(OwnerDispatchScope::Top);
        }
        if let Some(handle) = self.service_worker_child_client_handle(client_id) {
            return Some(OwnerDispatchScope::Child(handle));
        }
        self.service_worker_popup_clients
            .iter()
            .find_map(|(popup_id, popup_client_id)| {
                (*popup_client_id == client_id)
                    .then_some(OwnerDispatchScope::LightweightPopup(*popup_id))
            })
    }

    pub(crate) fn service_worker_window_client_completion_owner(
        &self,
        target: crate::types::ServiceWorkerWindowClientTarget,
    ) -> Option<ServiceWorkerWindowOwner> {
        let Some(dispatch_scope) = self.service_worker_window_client_owner(target.client_id) else {
            tracing::debug!(
                client_id = ?target.client_id,
                target_document_owner = ?target.document_owner,
                "dropped service worker completion for retired window client"
            );
            return None;
        };
        let Some(owner) = self.service_worker_window_owner_for_dispatch_scope(dispatch_scope)
        else {
            tracing::debug!(
                client_id = ?target.client_id,
                ?dispatch_scope,
                target_document_owner = ?target.document_owner,
                "dropped service worker completion for retired window document owner"
            );
            return None;
        };
        if owner.window_document_owner() != target.document_owner {
            tracing::debug!(
                client_id = ?target.client_id,
                ?dispatch_scope,
                window_document_owner = ?owner.window_document_owner(),
                target_document_owner = ?target.document_owner,
                "dropped stale service worker window client completion target"
            );
            return None;
        }
        Some(owner)
    }

    pub(crate) fn register_reserved_service_worker_top_level_client_for_navigation(
        &mut self,
        document_url: &Url,
    ) -> Option<ServiceWorkerClientId> {
        if !matches!(document_url.scheme(), "http" | "https") {
            return None;
        }
        let storage_key = service_worker_first_party_storage_key(document_url);
        Some(
            self.browser_context_runtime
                .register_reserved_service_worker_client(
                    document_url.clone(),
                    storage_key,
                    ServiceWorkerClientFrameType::TopLevel,
                    None,
                ),
        )
    }

    pub(crate) fn record_pending_service_worker_child_client_navigation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: crate::document_runtime::DomHandle,
        url: Url,
        continuation: crate::types::ServiceWorkerClientNavigateContinuation,
    ) -> Result<(), crate::service_worker_runtime::ServiceWorkerClientNavigateError> {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return Err(
                crate::service_worker_runtime::ServiceWorkerClientNavigateError::type_error(
                    "The client was not found.",
                ),
            );
        }
        if self.child_browsing_context_has_pending_navigation_or_document_load(handle) {
            return Err(
                crate::service_worker_runtime::ServiceWorkerClientNavigateError::type_error(
                    "The client is already navigating.",
                ),
            );
        }

        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        if !self.navigate_child_browsing_context_to_url(scope, handle, url.as_str()) {
            return Err(
                crate::service_worker_runtime::ServiceWorkerClientNavigateError::type_error(
                    "Cannot navigate to URL.",
                ),
            );
        }
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.set_pending_service_worker_client_navigation(continuation);
        }
        Ok(())
    }

    pub(crate) fn register_or_update_service_worker_popup_client(
        &mut self,
        document_owner: crate::native_bridge::LightweightPopupDocumentOwner,
        document_url: Url,
    ) -> Option<ServiceWorkerClientId> {
        if !self.lightweight_popup_document_owner_is_current(document_owner) {
            tracing::debug!(
                ?document_owner,
                %document_url,
                "ignored service worker client projection for stale popup document"
            );
            return None;
        }
        let popup_id = document_owner.popup_id();
        let storage_key = service_worker_first_party_storage_key(&document_url);
        if let Some(client_id) = self.service_worker_popup_clients.get(&popup_id).copied() {
            self.browser_context_runtime
                .update_service_worker_client_document(
                    client_id,
                    document_url,
                    storage_key,
                    ServiceWorkerClientFrameType::TopLevel,
                    Some(WindowDocumentOwner::LightweightPopup(document_owner)),
                );
            return Some(client_id);
        }
        let client_id = self.browser_context_runtime.register_service_worker_client(
            document_url,
            storage_key,
            ServiceWorkerClientFrameType::TopLevel,
            Some(WindowDocumentOwner::LightweightPopup(document_owner)),
            self.service_worker_task_sender(),
        );
        self.service_worker_popup_clients
            .insert(popup_id, client_id);
        Some(client_id)
    }

    pub(crate) fn update_service_worker_popup_client_if_registered(
        &mut self,
        popup_id: u64,
        document_url: Url,
    ) -> bool {
        let Some(client_id) = self.service_worker_popup_clients.get(&popup_id).copied() else {
            return false;
        };
        let Some(document_owner) = self.current_lightweight_popup_document_owner(popup_id) else {
            return false;
        };
        let storage_key = service_worker_first_party_storage_key(&document_url);
        self.browser_context_runtime
            .update_service_worker_client_document(
                client_id,
                document_url,
                storage_key,
                ServiceWorkerClientFrameType::TopLevel,
                Some(WindowDocumentOwner::LightweightPopup(document_owner)),
            )
    }

    pub(crate) fn register_or_update_service_worker_child_client(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) -> Option<ServiceWorkerClientId> {
        let document_url = self.child_browsing_context_current_url(handle)?;
        let storage_key = service_worker_first_party_storage_key(&document_url);
        let document_owner = self
            .frame_owner_store
            .current_child_document_task_owner(handle)
            .map(WindowDocumentOwner::Frame)?;
        let existing_client_id = self
            .child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.service_worker_client_id());
        if let Some(client_id) = existing_client_id
            && self
                .browser_context_runtime
                .update_service_worker_client_document_and_page_endpoint(
                    client_id,
                    document_url.clone(),
                    storage_key.clone(),
                    ServiceWorkerClientFrameType::Nested,
                    Some(document_owner),
                    self.service_worker_task_sender(),
                )
        {
            self.set_frame_owner_child_service_worker_client_id(handle, Some(client_id));
            return Some(client_id);
        }

        let client_id = self.browser_context_runtime.register_service_worker_client(
            document_url,
            storage_key,
            ServiceWorkerClientFrameType::Nested,
            Some(document_owner),
            self.service_worker_task_sender(),
        );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.set_service_worker_client_id(client_id);
            self.set_frame_owner_child_service_worker_client_id(handle, Some(client_id));
        }
        Some(client_id)
    }

    pub(crate) fn register_reserved_service_worker_child_client_for_navigation(
        &mut self,
        handle: crate::document_runtime::DomHandle,
        document_url: &Url,
    ) {
        if !matches!(document_url.scheme(), "http" | "https") {
            self.clear_pending_service_worker_child_client(handle);
            return;
        }
        if !self.child_browsing_contexts.contains_key(&handle) {
            return;
        }
        self.clear_pending_service_worker_child_client(handle);
        let storage_key = service_worker_first_party_storage_key(document_url);
        let client_id = self
            .browser_context_runtime
            .register_reserved_service_worker_client(
                document_url.clone(),
                storage_key,
                ServiceWorkerClientFrameType::Nested,
                None,
            );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.set_pending_service_worker_client_id(client_id);
        } else {
            self.browser_context_runtime
                .unregister_service_worker_client(client_id);
        }
    }

    pub(crate) fn promote_pending_service_worker_child_client(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) {
        let old_client_id = {
            let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
                return;
            };
            entry.promote_pending_service_worker_client_id()
        };
        if let Some(old_client_id) = old_client_id {
            self.browser_context_runtime
                .unregister_service_worker_client(old_client_id);
        }
        let current_client_id = self
            .child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.service_worker_client_id());
        self.set_frame_owner_child_service_worker_client_id(handle, current_client_id);
    }

    pub(crate) fn clear_pending_service_worker_child_client(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) {
        let Some(client_id) = self
            .child_browsing_contexts
            .get_mut(&handle)
            .and_then(|entry| entry.take_pending_service_worker_client_id())
        else {
            return;
        };
        self.browser_context_runtime
            .unregister_service_worker_client(client_id);
    }

    pub(crate) fn clear_pending_service_worker_child_client_if_matches(
        &mut self,
        handle: crate::document_runtime::DomHandle,
        expected_client_id: Option<ServiceWorkerClientId>,
    ) {
        let Some(expected_client_id) = expected_client_id else {
            return;
        };
        let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
            self.browser_context_runtime
                .unregister_service_worker_client(expected_client_id);
            return;
        };
        if entry.pending_service_worker_client_id() != Some(expected_client_id) {
            return;
        }
        let removed_client_id = entry
            .take_pending_service_worker_client_id()
            .expect("matching reserved child client must remain pending");
        self.browser_context_runtime
            .unregister_service_worker_client(removed_client_id);
    }

    fn set_frame_owner_child_service_worker_client_id(
        &mut self,
        handle: crate::document_runtime::DomHandle,
        client_id: Option<ServiceWorkerClientId>,
    ) {
        let _ = self
            .frame_owner_store
            .set_current_child_service_worker_client_id(handle, client_id);
    }

    pub(crate) fn unregister_service_worker_child_client(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) {
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The client was not found.".to_owned(),
        );
        self.clear_pending_service_worker_child_client(handle);
        let Some(client_id) = self
            .child_browsing_contexts
            .get_mut(&handle)
            .and_then(|entry| entry.take_service_worker_client_id())
        else {
            return;
        };
        self.set_frame_owner_child_service_worker_client_id(handle, None);
        self.browser_context_runtime
            .unregister_service_worker_client(client_id);
    }

    pub(crate) fn unregister_all_service_worker_child_clients(&mut self) {
        let handles = self
            .child_browsing_contexts
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for handle in handles {
            self.unregister_service_worker_child_client(handle);
        }
    }

    pub(crate) fn complete_pending_service_worker_child_client_navigation(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) {
        let Some((client_id, continuation)) = self
            .child_browsing_contexts
            .get_mut(&handle)
            .and_then(|entry| {
                entry.take_pending_service_worker_client_navigation_for_current_client()
            })
        else {
            return;
        };
        let result = self
            .browser_context_runtime
            .service_worker_runtime()
            .client_navigate_result_for_current_window_client(
                continuation.source_version_id,
                client_id,
            );
        self.browser_context_runtime
            .service_worker_runtime()
            .enqueue_client_navigate_completed(
                crate::types::ServiceWorkerClientNavigateCompletion {
                    request_id: continuation.request_id,
                    source_version_id: continuation.source_version_id,
                    source_run: continuation.source_run,
                    result,
                },
            );
    }

    pub(crate) fn reject_replaced_service_worker_child_client_navigation(
        &mut self,
        handle: crate::document_runtime::DomHandle,
        message: String,
    ) {
        self.clear_pending_service_worker_child_client(handle);
        let Some(continuation) = self
            .child_browsing_contexts
            .get_mut(&handle)
            .and_then(|entry| entry.take_pending_service_worker_client_navigation())
        else {
            return;
        };
        self.browser_context_runtime
            .service_worker_runtime()
            .enqueue_client_navigate_completed(
                crate::types::ServiceWorkerClientNavigateCompletion {
                    request_id: continuation.request_id,
                    source_version_id: continuation.source_version_id,
                    source_run: continuation.source_run,
                    result: Err(
                        crate::service_worker_runtime::ServiceWorkerClientNavigateError::type_error(
                            message,
                        ),
                    ),
                },
            );
    }

    pub(crate) fn service_worker_client_id_for_window_fetch(
        &self,
        owner_child_window: Option<crate::document_runtime::DomHandle>,
    ) -> ServiceWorkerClientId {
        owner_child_window
            .and_then(|handle| self.frame_owner_service_worker_client_id_for_child(handle))
            .unwrap_or(self.service_worker_client_id)
    }

    pub(crate) fn service_worker_client_id_for_worker_owner(
        &self,
        owner: WorkerOwnerScope,
    ) -> ServiceWorkerClientId {
        match owner {
            WorkerOwnerScope::Top => self.service_worker_client_id,
            WorkerOwnerScope::Child(handle) => self
                .frame_owner_service_worker_client_id_for_child(handle)
                .unwrap_or(self.service_worker_client_id),
            WorkerOwnerScope::LightweightPopup(popup_id) => self
                .service_worker_popup_clients
                .get(&popup_id)
                .copied()
                .unwrap_or(self.service_worker_client_id),
        }
    }

    pub(crate) fn service_worker_client_id_for_subresource_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> ServiceWorkerClientId {
        match owner {
            OwnerDispatchScope::Top => self.service_worker_client_id,
            OwnerDispatchScope::Child(handle) => self
                .frame_owner_service_worker_client_id_for_child(handle)
                .unwrap_or(self.service_worker_client_id),
            OwnerDispatchScope::LightweightPopup(popup_id) => self
                .service_worker_popup_clients
                .get(&popup_id)
                .copied()
                .unwrap_or(self.service_worker_client_id),
        }
    }

    fn frame_owner_service_worker_client_id_for_child(
        &self,
        handle: crate::document_runtime::DomHandle,
    ) -> Option<ServiceWorkerClientId> {
        self.frame_owner_current_child_snapshot(handle)
            .and_then(|snapshot| snapshot.settings.service_worker_client_id)
    }

    pub(crate) fn unregister_service_worker_popup_client(&mut self, popup_id: u64) {
        if let Some(client_id) = self.service_worker_popup_clients.remove(&popup_id) {
            self.browser_context_runtime
                .unregister_service_worker_client(client_id);
        }
        self.pending_service_worker_clients_open_window_popups
            .remove(&popup_id);
    }

    pub(crate) fn unregister_all_service_worker_popup_clients(&mut self) {
        for (_, client_id) in self.service_worker_popup_clients.drain() {
            self.browser_context_runtime
                .unregister_service_worker_client(client_id);
        }
        self.pending_service_worker_clients_open_window_popups
            .clear();
    }

    pub(crate) fn begin_service_worker_clients_open_window_popup(
        &mut self,
        popup_id: u64,
        document_url: Url,
        request_id: u64,
        source_version_id: ServiceWorkerVersionId,
        source_run: crate::runtime::RendererServiceWorkerRunIdentity,
    ) {
        let Some(document_owner) = self.current_lightweight_popup_document_owner(popup_id) else {
            tracing::warn!(
                popup_id,
                request_id,
                "resolved service worker clients.openWindow as null without a committed popup document"
            );
            self.browser_context_runtime
                .service_worker_runtime()
                .enqueue_clients_open_window_completed(
                    crate::types::ServiceWorkerClientsOpenWindowCompletion {
                        request_id,
                        source_version_id,
                        source_run,
                        result: Ok(None),
                    },
                );
            return;
        };
        let pending = PendingServiceWorkerClientsOpenWindowPopup {
            request_id,
            source_version_id,
            source_run,
            document_owner,
        };
        if self
            .register_or_update_service_worker_popup_client(document_owner, document_url)
            .is_none()
        {
            self.dispatch_service_worker_clients_open_window_popup_result(pending, Ok(None));
            return;
        }
        if self.lightweight_popup_has_pending_document_load(popup_id) {
            self.pending_service_worker_clients_open_window_popups
                .insert(popup_id, pending);
            return;
        }
        let result = self.service_worker_clients_open_window_popup_result(&pending);
        self.dispatch_service_worker_clients_open_window_popup_result(pending, result);
    }

    pub(crate) fn finish_service_worker_clients_open_window_popup(
        &mut self,
        document_owner: crate::native_bridge::LightweightPopupDocumentOwner,
    ) {
        let popup_id = document_owner.popup_id();
        let Some(pending) = self
            .pending_service_worker_clients_open_window_popups
            .get(&popup_id)
            .filter(|pending| pending.document_owner == document_owner)
            .cloned()
        else {
            return;
        };
        self.pending_service_worker_clients_open_window_popups
            .remove(&popup_id);
        let result = self.service_worker_clients_open_window_popup_result(&pending);
        self.dispatch_service_worker_clients_open_window_popup_result(pending, result);
    }

    pub(crate) fn finish_service_worker_clients_open_window_popup_with_null_for_owner(
        &mut self,
        document_owner: crate::native_bridge::LightweightPopupDocumentOwner,
    ) {
        let popup_id = document_owner.popup_id();
        let Some(pending) = self
            .pending_service_worker_clients_open_window_popups
            .get(&popup_id)
            .filter(|pending| pending.document_owner == document_owner)
            .cloned()
        else {
            return;
        };
        self.pending_service_worker_clients_open_window_popups
            .remove(&popup_id);
        self.dispatch_service_worker_clients_open_window_popup_result(pending, Ok(None));
    }

    fn service_worker_clients_open_window_popup_result(
        &self,
        pending: &PendingServiceWorkerClientsOpenWindowPopup,
    ) -> Result<Option<ServiceWorkerClientSnapshot>, ServiceWorkerClientsOpenWindowError> {
        if !self.lightweight_popup_document_owner_is_current(pending.document_owner) {
            tracing::debug!(
                document_owner = ?pending.document_owner,
                "resolved stale service worker clients.openWindow popup as null"
            );
            return Ok(None);
        }
        let popup_id = pending.document_owner.popup_id();
        let Some(client_id) = self.service_worker_popup_clients.get(&popup_id).copied() else {
            return Ok(None);
        };
        self.browser_context_runtime
            .service_worker_runtime()
            .client_navigate_result_for_current_window_client(pending.source_version_id, client_id)
            .map_err(|error| match error {
                crate::service_worker_runtime::ServiceWorkerClientNavigateError::TypeError(
                    message,
                ) => ServiceWorkerClientsOpenWindowError::type_error(message),
            })
    }

    fn dispatch_service_worker_clients_open_window_popup_result(
        &self,
        pending: PendingServiceWorkerClientsOpenWindowPopup,
        result: Result<Option<ServiceWorkerClientSnapshot>, ServiceWorkerClientsOpenWindowError>,
    ) {
        self.browser_context_runtime
            .service_worker_runtime()
            .enqueue_clients_open_window_completed(
                crate::types::ServiceWorkerClientsOpenWindowCompletion {
                    request_id: pending.request_id,
                    source_version_id: pending.source_version_id,
                    source_run: pending.source_run,
                    result,
                },
            );
    }

    pub(crate) fn service_worker_registration_for_client(
        &self,
        request_context: &ServiceWorkerWindowRequestContext,
        client_url: &Url,
    ) -> Option<ServiceWorkerRegistrationSnapshot> {
        self.browser_context_runtime
            .service_worker_registration_for_client(
                client_url,
                &request_context.serialized_storage_key(),
            )
    }

    pub(crate) fn service_worker_registrations(
        &self,
        request_context: &ServiceWorkerWindowRequestContext,
    ) -> Vec<ServiceWorkerRegistrationSnapshot> {
        self.browser_context_runtime.service_worker_registrations(
            request_context.document_url(),
            &request_context.serialized_storage_key(),
        )
    }

    pub(crate) fn service_worker_controller_for_fetch(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: &Url,
        request_url: &Url,
    ) -> Option<ServiceWorkerControlState> {
        if self
            .current_main_document_resource_loader()
            .is_some_and(|loader| loader.request_client().bypass_service_worker())
        {
            return None;
        }
        self.browser_context_runtime
            .service_worker_controller_for_fetch(client_id, request_url)
            .or_else(|| {
                self.service_worker_control
                    .as_ref()
                    .filter(|state| state.controls_document(document_url))
                    .cloned()
            })
    }

    pub(crate) fn dispatch_service_worker_fetch(
        &self,
        dispatch: ServiceWorkerFetchDispatch,
    ) -> bool {
        self.browser_context_runtime
            .dispatch_service_worker_fetch(dispatch)
    }

    pub(crate) fn dispatch_service_worker_message(
        &self,
        version_id: ServiceWorkerVersionId,
        owner: OwnerDispatchScope,
        payload: V8StructuredClonePayload,
    ) -> bool {
        let Some((source_client_id, source_document_url)) = (match owner {
            OwnerDispatchScope::Top => {
                Some((self.service_worker_client_id, self.document_url().clone()))
            }
            OwnerDispatchScope::Child(handle) => self
                .child_browsing_contexts
                .get(&handle)
                .and_then(|entry| entry.service_worker_client_id())
                .zip(self.child_browsing_context_current_url(handle)),
            OwnerDispatchScope::LightweightPopup(popup_id) => self
                .service_worker_popup_clients
                .get(&popup_id)
                .copied()
                .zip(self.lightweight_popup_document_url(popup_id)),
        }) else {
            return false;
        };
        let source_origin = Some(moli_url::origin_ascii_serialization(&source_document_url));
        self.browser_context_runtime
            .dispatch_service_worker_message(version_id, source_client_id, source_origin, payload)
    }

    pub(crate) fn service_worker_fetch_request(
        &self,
        client_id: ServiceWorkerClientId,
        url: Url,
        method: String,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
        destination: ServiceWorkerRequestDestination,
        request_mode: moli_fetch::RequestMode,
        credentials_mode: moli_fetch::RequestCredentialsMode,
        redirect_mode: moli_fetch::RequestRedirectMode,
        priority: Option<moli_fetch::FetchPriorityHint>,
        metadata: ServiceWorkerFetchRequestMetadata,
    ) -> ServiceWorkerFetchRequest {
        ServiceWorkerFetchRequest {
            client_id,
            resulting_client_id: None,
            url,
            method,
            headers,
            body,
            destination,
            request_mode,
            credentials_mode,
            redirect_mode,
            priority,
            is_reload: false,
            metadata,
        }
    }
}
