//! Service Worker API request ids, pending promises and result delivery.

use super::*;

pub(in crate::worker) struct PendingServiceWorkerClientQuery {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
    pub(in crate::worker) query_type: PendingServiceWorkerClientQueryType,
}

pub(in crate::worker) struct PendingServiceWorkerClientNavigate {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerClientFocus {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerClientsOpenWindow {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerShowNotification {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerGetNotifications {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerSyncRegistration {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerSyncGetTags {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerPeriodicSyncRegistration {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerPeriodicSyncGetTags {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerPeriodicSyncUnregistration {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerPushSubscribe {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerPushGetSubscription {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

pub(in crate::worker) struct PendingServiceWorkerPushUnsubscribe {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::worker) enum PendingServiceWorkerClientQueryType {
    Get,
    MatchAll,
}

/// Checked allocator for one operation-local Service Worker request namespace.
///
/// The parent/worker protocol intentionally gives each operation its own
/// namespace. Keeping that shape avoids coupling unrelated APIs while making
/// exhaustion fail instead of overwriting a live pending resolver.
#[derive(Debug)]
pub(in crate::worker) struct WorkerServiceWorkerRequestIdAllocator {
    next: u64,
}

impl Default for WorkerServiceWorkerRequestIdAllocator {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl WorkerServiceWorkerRequestIdAllocator {
    fn allocate(&mut self) -> u64 {
        let request_id = self.next;
        self.next = request_id
            .checked_add(1)
            .expect("worker Service Worker request id space exhausted");
        request_id
    }
}

impl WorkerGlobalState {
    pub(in crate::worker) fn register_pending_service_worker_client_query(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
        query_type: PendingServiceWorkerClientQueryType,
    ) -> u64 {
        let request_id = self.service_worker_client_query_request_ids.allocate();
        self.pending_service_worker_client_queries.insert(
            request_id,
            PendingServiceWorkerClientQuery {
                resolver,
                query_type,
            },
        );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_client_query(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerClientQuery> {
        self.pending_service_worker_client_queries
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_client_navigate(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_client_navigate_request_ids.allocate();
        self.pending_service_worker_client_navigates
            .insert(request_id, PendingServiceWorkerClientNavigate { resolver });
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_client_navigate(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerClientNavigate> {
        self.pending_service_worker_client_navigates
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_client_focus(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_client_focus_request_ids.allocate();
        self.pending_service_worker_client_focuses
            .insert(request_id, PendingServiceWorkerClientFocus { resolver });
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_client_focus(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerClientFocus> {
        self.pending_service_worker_client_focuses
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_clients_open_window(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_clients_open_window_request_ids
            .allocate();
        self.pending_service_worker_clients_open_windows.insert(
            request_id,
            PendingServiceWorkerClientsOpenWindow { resolver },
        );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_clients_open_window(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerClientsOpenWindow> {
        self.pending_service_worker_clients_open_windows
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_show_notification(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_show_notification_request_ids.allocate();
        self.pending_service_worker_show_notifications.insert(
            request_id,
            PendingServiceWorkerShowNotification { resolver },
        );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_show_notification(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerShowNotification> {
        self.pending_service_worker_show_notifications
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_get_notifications(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_get_notifications_request_ids.allocate();
        self.pending_service_worker_get_notifications.insert(
            request_id,
            PendingServiceWorkerGetNotifications { resolver },
        );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_get_notifications(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerGetNotifications> {
        self.pending_service_worker_get_notifications
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_sync_registration(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_sync_registration_request_ids.allocate();
        self.pending_service_worker_sync_registrations.insert(
            request_id,
            PendingServiceWorkerSyncRegistration { resolver },
        );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_sync_registration(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerSyncRegistration> {
        self.pending_service_worker_sync_registrations
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_sync_get_tags(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_sync_get_tags_request_ids.allocate();
        self.pending_service_worker_sync_get_tags
            .insert(request_id, PendingServiceWorkerSyncGetTags { resolver });
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_sync_get_tags(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerSyncGetTags> {
        self.pending_service_worker_sync_get_tags
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_periodic_sync_registration(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_periodic_sync_registration_request_ids
            .allocate();
        self.pending_service_worker_periodic_sync_registrations
            .insert(
                request_id,
                PendingServiceWorkerPeriodicSyncRegistration { resolver },
            );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_periodic_sync_registration(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPeriodicSyncRegistration> {
        self.pending_service_worker_periodic_sync_registrations
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_periodic_sync_get_tags(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_periodic_sync_get_tags_request_ids
            .allocate();
        self.pending_service_worker_periodic_sync_get_tags.insert(
            request_id,
            PendingServiceWorkerPeriodicSyncGetTags { resolver },
        );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_periodic_sync_get_tags(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPeriodicSyncGetTags> {
        self.pending_service_worker_periodic_sync_get_tags
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_periodic_sync_unregistration(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_periodic_sync_unregistration_request_ids
            .allocate();
        self.pending_service_worker_periodic_sync_unregistrations
            .insert(
                request_id,
                PendingServiceWorkerPeriodicSyncUnregistration { resolver },
            );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_periodic_sync_unregistration(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPeriodicSyncUnregistration> {
        self.pending_service_worker_periodic_sync_unregistrations
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_push_subscribe(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_push_subscription_request_ids.allocate();
        self.pending_service_worker_push_subscriptions
            .insert(request_id, PendingServiceWorkerPushSubscribe { resolver });
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_push_subscribe(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPushSubscribe> {
        self.pending_service_worker_push_subscriptions
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_push_get_subscription(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_push_get_subscription_request_ids
            .allocate();
        self.pending_service_worker_push_get_subscriptions.insert(
            request_id,
            PendingServiceWorkerPushGetSubscription { resolver },
        );
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_push_get_subscription(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPushGetSubscription> {
        self.pending_service_worker_push_get_subscriptions
            .remove(&request_id)
    }

    pub(in crate::worker) fn register_pending_service_worker_push_unsubscribe(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_push_unsubscription_request_ids
            .allocate();
        self.pending_service_worker_push_unsubscriptions
            .insert(request_id, PendingServiceWorkerPushUnsubscribe { resolver });
        request_id
    }

    pub(in crate::worker) fn take_pending_service_worker_push_unsubscribe(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPushUnsubscribe> {
        self.pending_service_worker_push_unsubscriptions
            .remove(&request_id)
    }

    pub(in crate::worker) fn consume_service_worker_window_interaction(&mut self) -> bool {
        if self.service_worker_window_interaction_allowed_count == 0 {
            return false;
        }

        self.service_worker_window_interaction_allowed_count = self
            .service_worker_window_interaction_allowed_count
            .saturating_sub(1);

        if let Some((_, pending)) = self
            .pending_service_worker_message_events
            .iter_mut()
            .find(|(_, pending)| pending.window_interaction_allowed)
        {
            pending.window_interaction_allowed = false;
            return true;
        }

        if let Some((_, pending)) = self
            .pending_service_worker_notification_events
            .iter_mut()
            .find(|(_, pending)| pending.window_interaction_allowed)
        {
            pending.window_interaction_allowed = false;
        }

        true
    }
}

pub(in crate::worker) fn drain_service_worker_client_query_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerClientQueryResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_client_query(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match pending.query_type {
        PendingServiceWorkerClientQueryType::Get => {
            let value = result
                .clients
                .into_iter()
                .next()
                .and_then(|client| build_service_worker_client_object_from_snapshot(scope, &client))
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::undefined(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        PendingServiceWorkerClientQueryType::MatchAll => {
            let array = v8::Array::new(scope, result.clients.len() as i32);
            for (index, client) in result.clients.iter().enumerate() {
                if let Some(object) =
                    build_service_worker_client_object_from_snapshot(scope, client)
                {
                    let _ = array.set_index(scope, index as u32, object.into());
                }
            }
            let _ = resolver.resolve(scope, array.into());
        }
    }
}

pub(in crate::worker) fn drain_service_worker_client_navigate_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerClientNavigateResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_client_navigate(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(Some(client)) => {
            let value = build_service_worker_client_object_from_snapshot(scope, &client)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Ok(None) => {
            let _ = resolver.resolve(scope, v8::null(scope).into());
        }
        Err(error) => match error {
            ServiceWorkerClientNavigateError::TypeError(message) => {
                let Some(message) = v8_string(scope, &message) else {
                    let _ = resolver.reject(scope, v8::undefined(scope).into());
                    return;
                };
                let error = v8::Exception::type_error(scope, message);
                let _ = resolver.reject(scope, error);
            }
        },
    }
}

pub(in crate::worker) fn drain_service_worker_client_focus_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: crate::runtime::ServiceWorkerClientFocusResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_client_focus(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(client) => {
            let value = build_service_worker_client_object_from_snapshot(scope, &client)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Err(error) => {
            reject_service_worker_client_focus_error(scope, resolver, error);
        }
    }
}

fn reject_service_worker_client_focus_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    error: ServiceWorkerClientFocusError,
) {
    match error {
        ServiceWorkerClientFocusError::DomException { name, message } => {
            let reason = worker_dom_exception_value(scope, &message, name);
            let _ = resolver.reject(scope, reason);
        }
        ServiceWorkerClientFocusError::TypeError(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let reason = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, reason);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_clients_open_window_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerClientsOpenWindowResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_clients_open_window(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(Some(client)) => {
            let value = build_service_worker_client_object_from_snapshot(scope, &client)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Ok(None) => {
            let _ = resolver.resolve(scope, v8::null(scope).into());
        }
        Err(error) => match error {
            ServiceWorkerClientsOpenWindowError::TypeError(message) => {
                let Some(message) = v8_string(scope, &message) else {
                    let _ = resolver.reject(scope, v8::undefined(scope).into());
                    return;
                };
                let error = v8::Exception::type_error(scope, message);
                let _ = resolver.reject(scope, error);
            }
        },
    }
}

pub(in crate::worker) fn drain_service_worker_show_notification_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerShowNotificationResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_show_notification(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_get_notifications_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerGetNotificationsResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_get_notifications(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    let notifications = match result.result {
        Ok(notifications) => notifications,
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
            return;
        }
    };
    let array = v8::Array::new(scope, notifications.len() as i32);
    for (index, notification) in notifications.iter().enumerate() {
        if let Some(object) =
            crate::context_bootstrap::build_notification_object_from_snapshot(scope, notification)
        {
            let _ = array.set_index(scope, index as u32, object.into());
        }
    }
    let _ = resolver.resolve(scope, array.into());
}

pub(in crate::worker) fn drain_service_worker_sync_registration_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerSyncRegistrationResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_sync_registration(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_sync_get_tags_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerSyncGetTagsResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_sync_get_tags(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(tags) => {
            let array = v8::Array::new(scope, tags.len() as i32);
            for (index, tag) in tags.iter().enumerate() {
                if let Some(value) = v8_string(scope, tag) {
                    let _ = array.set_index(scope, index as u32, value.into());
                }
            }
            let _ = resolver.resolve(scope, array.into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_periodic_sync_registration_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: crate::runtime::ServiceWorkerPeriodicSyncRegistrationResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_periodic_sync_registration(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_periodic_sync_get_tags_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: crate::runtime::ServiceWorkerPeriodicSyncGetTagsResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_periodic_sync_get_tags(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(tags) => {
            let array = v8::Array::new(scope, tags.len() as i32);
            for (index, tag) in tags.iter().enumerate() {
                if let Some(value) = v8_string(scope, tag) {
                    let _ = array.set_index(scope, index as u32, value.into());
                }
            }
            let _ = resolver.resolve(scope, array.into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_periodic_sync_unregistration_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: crate::runtime::ServiceWorkerPeriodicSyncUnregistrationResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_periodic_sync_unregistration(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_push_subscribe_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerPushSubscribeResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_push_subscribe(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(subscription) => {
            let value = build_service_worker_push_subscription_object(scope, &subscription)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_push_get_subscription_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerPushGetSubscriptionResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_push_get_subscription(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(Some(subscription)) => {
            let value = build_service_worker_push_subscription_object(scope, &subscription)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Ok(None) => {
            let _ = resolver.resolve(scope, v8::null(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(in crate::worker) fn drain_service_worker_push_unsubscribe_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerPushUnsubscribeResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_push_unsubscribe(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(unsubscribed) => {
            let _ = resolver.resolve(scope, v8::Boolean::new(scope, unsubscribed).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

#[cfg(test)]
mod service_worker_request_id_allocator_tests {
    use super::WorkerServiceWorkerRequestIdAllocator;

    #[test]
    #[should_panic(expected = "worker Service Worker request id space exhausted")]
    fn operation_local_request_ids_never_wrap() {
        let mut ids = WorkerServiceWorkerRequestIdAllocator { next: u64::MAX };
        let _ = ids.allocate();
    }
}

pub(in crate::worker) struct PendingServiceWorkerUpdate {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
    pub(in crate::worker) registration: v8::Global<v8::Object>,
}

pub(in crate::worker) fn drain_service_worker_update_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    request_id: u64,
    result: Result<
        crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot,
        crate::service_worker_runtime::ServiceWorkerRegistrationError,
    >,
) {
    let Some(pending) = state
        .borrow_mut()
        .pending_service_worker_updates
        .remove(&request_id)
    else {
        return;
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result {
        Ok(_) => {
            let registration = v8::Local::new(scope, &pending.registration);
            let _ = resolver.resolve(scope, registration.into());
        }
        Err(error) => {
            let exception = if error.kind.rejects_as_type_error_for_update() {
                let message = v8_string(scope, &error.message)
                    .unwrap_or_else(|| v8str(scope, "ServiceWorker update failed"));
                v8::Exception::type_error(scope, message)
            } else {
                crate::context_bootstrap::new_dom_exception_value(
                    scope,
                    &error.message,
                    error.kind.dom_exception_name(),
                )
            };
            let _ = resolver.reject(scope, exception);
        }
    }
}
