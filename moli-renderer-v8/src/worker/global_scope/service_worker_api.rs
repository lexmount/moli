use super::*;

pub(super) fn install_service_worker_global_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    registration_id: crate::runtime::ServiceWorkerRegistrationId,
    version_id: crate::runtime::ServiceWorkerVersionId,
    scope_url: &Url,
) -> Result<()> {
    let registration =
        build_service_worker_global_registration(scope, registration_id, version_id, scope_url)?;
    let clients = ServiceWorkerClientsDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to build service worker clients: {error}"))?;
    ServiceWorkerGlobalRuntimeDeclaration::new(registration, clients)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize service worker global: {error}"))
}

fn build_service_worker_global_registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration_id: crate::runtime::ServiceWorkerRegistrationId,
    version_id: crate::runtime::ServiceWorkerVersionId,
    scope_url: &Url,
) -> Result<v8::Local<'s, v8::Object>> {
    let sync_manager = ServiceWorkerGlobalSyncManagerDeclaration {
        register: (),
        get_tags: (),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build service worker sync manager: {error:?}"))?;
    let periodic_sync_manager = ServiceWorkerGlobalPeriodicSyncManagerDeclaration {
        register: (),
        get_tags: (),
        unregister: (),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build service worker periodic sync manager: {error:?}"))?;
    let push_manager = crate::context_bootstrap::push_interfaces::build_manager(scope)
        .ok_or_else(|| anyhow!("failed to build service worker push manager"))?;
    let navigation_preload =
        build_service_worker_global_navigation_preload_manager(scope, scope_url)?;
    let prototype = crate::context_bootstrap::ensure_intrinsic_interface_prototype(
        scope,
        "ServiceWorkerRegistration",
    )?;
    let update_via_cache = worker_service_worker_runtime(scope)
        .and_then(|runtime| runtime.registration_snapshot_by_id(registration_id))
        .map(|snapshot| snapshot.update_via_cache().as_str())
        .unwrap_or("imports");
    let registration = ServiceWorkerRegistrationObjectDeclaration {
        prototype,
        scope: scope_url.as_str().to_owned(),
        update_via_cache,
        sync: Some(sync_manager),
        periodic_sync: Some(periodic_sync_manager),
        push_manager: Some(push_manager),
        navigation_preload: Some(navigation_preload),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build service worker registration: {error:?}"))?;
    crate::context_bootstrap::mark_simple_event_target_slot(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_EVENTS_SLOT,
    );
    install_simple_event_target_ordered_handlers(scope, registration);
    let scope_value = v8_string(scope, scope_url.as_str())
        .ok_or_else(|| anyhow!("failed to allocate service worker registration scope"))?;
    set_private_value(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_SCOPE_SLOT,
        scope_value.into(),
    );
    let registration_id_value = v8::BigInt::new_from_u64(scope, registration_id.as_u64());
    set_private_value(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_ID_SLOT,
        registration_id_value.into(),
    );
    let version_id_value = v8::BigInt::new_from_u64(scope, version_id.as_u64());
    set_private_value(
        scope,
        registration,
        SERVICE_WORKER_VERSION_ID_SLOT,
        version_id_value.into(),
    );
    Ok(registration)
}

fn build_service_worker_global_navigation_preload_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    scope_url: &Url,
) -> Result<v8::Local<'s, v8::Object>> {
    let prototype = crate::context_bootstrap::ensure_intrinsic_interface_prototype(
        scope,
        "NavigationPreloadManager",
    )?;
    let navigation_preload = NavigationPreloadManagerObjectDeclaration::new(prototype)
        .bind(scope)
        .map_err(|error| anyhow!("failed to build navigation preload manager: {error:?}"))?;
    let scope_value = v8_string(scope, scope_url.as_str())
        .ok_or_else(|| anyhow!("failed to allocate navigation preload registration scope"))?;
    set_private_value(
        scope,
        navigation_preload,
        SERVICE_WORKER_NAVIGATION_PRELOAD_MANAGER_SCOPE_SLOT,
        scope_value.into(),
    );
    Ok(navigation_preload)
}

enum ServiceWorkerRegistrationWorkerPhase {
    Installing,
    Waiting,
    Active,
}

pub(super) fn service_worker_registration_installing_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(service_worker_registration_worker_value(
        scope,
        ServiceWorkerRegistrationWorkerPhase::Installing,
    ));
}

pub(super) fn service_worker_registration_waiting_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(service_worker_registration_worker_value(
        scope,
        ServiceWorkerRegistrationWorkerPhase::Waiting,
    ));
}

pub(super) fn service_worker_registration_active_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(service_worker_registration_worker_value(
        scope,
        ServiceWorkerRegistrationWorkerPhase::Active,
    ));
}

fn service_worker_registration_worker_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    phase: ServiceWorkerRegistrationWorkerPhase,
) -> v8::Local<'s, v8::Value> {
    let Some((registration_id, _, _)) = service_worker_runtime_identity(scope) else {
        return v8::null(scope).into();
    };
    let Some(snapshot) = worker_service_worker_runtime(scope)
        .and_then(|runtime| runtime.registration_snapshot_by_id(registration_id))
    else {
        return v8::null(scope).into();
    };
    let version = match phase {
        ServiceWorkerRegistrationWorkerPhase::Installing => snapshot.installing(),
        ServiceWorkerRegistrationWorkerPhase::Waiting => snapshot.waiting(),
        ServiceWorkerRegistrationWorkerPhase::Active => snapshot.active(),
    };
    version
        .and_then(|version| build_service_worker_global_service_worker(scope, version).ok())
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into())
}

pub(in crate::worker) fn build_service_worker_global_service_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    version: &crate::service_worker_runtime::ServiceWorkerVersionSnapshot,
) -> Result<v8::Local<'s, v8::Object>> {
    let prototype =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, "ServiceWorker")?;
    let worker = ServiceWorkerObjectDeclaration {
        prototype,
        script_url: version.script_url().as_str().to_owned(),
        state: version.state(),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build worker ServiceWorker object: {error:?}"))?;
    let version_id_value = v8::BigInt::new_from_u64(scope, version.version_id().as_u64());
    set_private_value(
        scope,
        worker,
        SERVICE_WORKER_VERSION_ID_SLOT,
        version_id_value.into(),
    );
    crate::context_bootstrap::mark_simple_event_target_slot(
        scope,
        worker,
        SERVICE_WORKER_WORKER_EVENTS_SLOT,
    );
    install_simple_event_target_ordered_handlers(scope, worker);
    Ok(worker)
}

pub(super) fn service_worker_registration_unregister_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(resolved_worker_promise(scope, v8::Boolean::new(scope, true).into()).into());
}

pub(super) fn service_worker_registration_show_notification_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(title) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("ServiceWorkerRegistration.showNotification", 1),
        "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(options) = crate::context_bootstrap::notification_options_payload(scope, args.get(1))
    else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let permission_state = {
        let state = state.borrow();
        worker_permission_state(&state, "notifications")
    };
    if permission_state != "granted" {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': notification permission has not been granted.",
                ),
            ),
        );
        return;
    }
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_show_notification(v8::Global::new(scope, resolver))
    };
    if parent_tx
        .send(WorkerToParentMessage::ServiceWorkerShowNotification(
            crate::runtime::ServiceWorkerShowNotification {
                request_id,
                registration_id,
                version_id,
                title: title.0,
                tag: options.tag,
                metadata: options.metadata,
                actions: options.actions,
                data: options.data,
            },
        ))
        .is_err()
    {
        let pending = {
            let mut state = state.borrow_mut();
            state.take_pending_service_worker_show_notification(request_id)
        };
        if let Some(pending) = pending {
            let resolver = v8::Local::new(scope, &pending.resolver);
            let _ = resolver.reject(
                scope,
                v8::Exception::type_error(
                    scope,
                    v8str(
                        scope,
                        "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': Service Worker runtime is unavailable.",
                    ),
                ),
            );
        }
    }
}

pub(super) fn service_worker_registration_get_notifications_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = crate::context_bootstrap::notification_get_options_tag(scope, args.get(0))
    else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getNotifications' on 'ServiceWorkerRegistration': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getNotifications' on 'ServiceWorkerRegistration': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_get_notifications(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerGetNotifications(
        crate::runtime::ServiceWorkerGetNotifications {
            request_id,
            registration_id,
            version_id,
            tag,
        },
    ));
}

pub(super) fn service_worker_sync_manager_register_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("SyncManager.register", 1),
        "Failed to execute 'register' on 'SyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'register' on 'SyncManager': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'register' on 'SyncManager': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let permission_state = {
        let state = state.borrow();
        service_worker_background_sync_permission_state(&state)
    };
    if permission_state != "granted" {
        let reason = worker_dom_exception_value(
            scope,
            "Background Sync permission has not been granted.",
            "NotAllowedError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_sync_registration(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerSyncRegistration(
        crate::runtime::ServiceWorkerSyncRegistration {
            request_id,
            registration_id,
            version_id,
            tag: tag.0,
        },
    ));
}

pub(super) fn service_worker_sync_manager_get_tags_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getTags' on 'SyncManager': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getTags' on 'SyncManager': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_sync_get_tags(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerSyncGetTags(
        crate::runtime::ServiceWorkerSyncGetTags {
            request_id,
            registration_id,
            version_id,
        },
    ));
}

pub(super) fn service_worker_periodic_sync_manager_register_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("PeriodicSyncManager.register", 1),
        "Failed to execute 'register' on 'PeriodicSyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(options) = service_worker_periodic_sync_options(scope, args.get(1)) else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'register' on 'PeriodicSyncManager': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'register' on 'PeriodicSyncManager': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let permission_state = {
        let state = state.borrow();
        service_worker_periodic_sync_permission_state(&state)
    };
    if permission_state != "granted" {
        let reason = worker_dom_exception_value(
            scope,
            "Periodic Background Sync permission has not been granted.",
            "NotAllowedError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_periodic_sync_registration(v8::Global::new(
            scope, resolver,
        ))
    };
    let _ = parent_tx.send(
        WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(
            crate::runtime::ServiceWorkerPeriodicSyncRegistration {
                request_id,
                registration_id,
                version_id,
                tag: tag.0,
                min_interval_ms: options.min_interval,
            },
        ),
    );
}

pub(super) fn service_worker_periodic_sync_manager_get_tags_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getTags' on 'PeriodicSyncManager': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getTags' on 'PeriodicSyncManager': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_periodic_sync_get_tags(v8::Global::new(
            scope, resolver,
        ))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(
        crate::runtime::ServiceWorkerPeriodicSyncGetTags {
            request_id,
            registration_id,
            version_id,
        },
    ));
}

pub(super) fn service_worker_periodic_sync_manager_unregister_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("PeriodicSyncManager.unregister", 1),
        "Failed to execute 'unregister' on 'PeriodicSyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_periodic_sync_unregistration(v8::Global::new(
            scope, resolver,
        ))
    };
    let _ = parent_tx.send(
        WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(
            crate::runtime::ServiceWorkerPeriodicSyncUnregistration {
                request_id,
                registration_id,
                version_id,
                tag: tag.0,
            },
        ),
    );
}

fn service_worker_periodic_sync_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<BackgroundSyncOptions> {
    match webidl::parse_dictionary::<BackgroundSyncOptions>(
        scope,
        value,
        webidl::Context::argument("PeriodicSyncManager.register", 2),
    ) {
        Ok(options) => Some(options.unwrap_or_default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn service_worker_background_sync_permission_state(state: &WorkerGlobalState) -> String {
    if !state.secure_context {
        return "denied".to_owned();
    }
    match worker_permission_state(state, "background-sync").as_str() {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

fn service_worker_periodic_sync_permission_state(state: &WorkerGlobalState) -> String {
    if !state.secure_context {
        return "denied".to_owned();
    }
    match worker_permission_state(state, "periodic-background-sync").as_str() {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

pub(super) fn service_worker_navigation_preload_manager_enable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    service_worker_navigation_preload_manager_set_enabled(scope, args, rv, true);
}

pub(super) fn service_worker_navigation_preload_manager_disable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    service_worker_navigation_preload_manager_set_enabled(scope, args, rv, false);
}

fn service_worker_navigation_preload_manager_set_enabled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    enabled: bool,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute navigation preload operation: registration scope is unavailable.",
        );
        return;
    };
    let Some(runtime) = worker_service_worker_runtime(scope) else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute navigation preload operation: Service Worker runtime is unavailable.",
        );
        return;
    };
    match runtime.set_navigation_preload_enabled_for_scope(&scope_url, enabled) {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(error) => reject_worker_navigation_preload_state_error(scope, resolver, error),
    }
}

pub(super) fn service_worker_navigation_preload_manager_set_header_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let header_value = match service_worker_navigation_preload_header_value(scope, &args) {
        Ok(header_value) => header_value,
        Err(error) => {
            reject_worker_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    if !service_worker_navigation_preload_valid_header_value(&header_value) {
        reject_worker_type_error(
            scope,
            resolver,
            "The string provided to setHeaderValue is not a valid HTTP header field value.",
        );
        return;
    }
    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute 'setHeaderValue' on 'NavigationPreloadManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(runtime) = worker_service_worker_runtime(scope) else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute 'setHeaderValue' on 'NavigationPreloadManager': Service Worker runtime is unavailable.",
        );
        return;
    };
    match runtime.set_navigation_preload_header_value_for_scope(&scope_url, header_value) {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(error) => reject_worker_navigation_preload_state_error(scope, resolver, error),
    }
}

pub(super) fn service_worker_navigation_preload_manager_get_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute 'getState' on 'NavigationPreloadManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(runtime) = worker_service_worker_runtime(scope) else {
        reject_worker_navigation_preload_state_error(
            scope,
            resolver,
            ServiceWorkerNavigationPreloadStateError::InvalidState,
        );
        return;
    };
    let Some(state) = runtime.navigation_preload_state_for_scope(&scope_url) else {
        reject_worker_navigation_preload_state_error(
            scope,
            resolver,
            ServiceWorkerNavigationPreloadStateError::InvalidState,
        );
        return;
    };
    let state_object = build_worker_navigation_preload_state_object(scope, &state);
    let _ = resolver.resolve(scope, state_object.into());
}

fn worker_service_worker_runtime(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::service_worker_runtime::ServiceWorkerRuntimeService> {
    let state = get_worker_state(scope)?;
    state.borrow().service_worker_runtime.clone()
}

fn service_worker_navigation_preload_manager_scope_from_this<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    this: v8::Local<'s, v8::Object>,
) -> Option<Url> {
    let value = get_private_value(
        scope,
        this,
        SERVICE_WORKER_NAVIGATION_PRELOAD_MANAGER_SCOPE_SLOT,
    )?;
    let scope_string = value.to_string(scope)?.to_rust_string_lossy(scope);
    Url::parse(&scope_string).ok()
}

fn service_worker_navigation_preload_header_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<String, webidl::WebIdlError> {
    let context = webidl::Context::argument("NavigationPreloadManager.setHeaderValue", 1);
    if args.length() <= 0 {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::ByteString>(scope, args.get(0), context).map(Into::into)
}

fn service_worker_navigation_preload_valid_header_value(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch as u32 <= 0xff && !matches!(ch, '\0' | '\r' | '\n'))
}

fn build_worker_navigation_preload_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &ServiceWorkerNavigationPreloadState,
) -> v8::Local<'s, v8::Object> {
    let object = v8::Object::new(scope);
    let _ = WorkerNavigationPreloadStateDeclaration::new(state.enabled, state.header_value.clone())
        .initialize(scope, object);
    object
}

fn reject_worker_navigation_preload_state_error(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    error: ServiceWorkerNavigationPreloadStateError,
) {
    match error {
        ServiceWorkerNavigationPreloadStateError::InvalidState => {
            let reason = worker_dom_exception_value(
                scope,
                "Registration failed - no active Service Worker",
                "InvalidStateError",
            );
            let _ = resolver.reject(scope, reason);
        }
        ServiceWorkerNavigationPreloadStateError::StorageFailure => {
            reject_worker_type_error(
                scope,
                resolver,
                "Failed to persist navigation preload state.",
            );
        }
    }
}

fn reject_worker_type_error(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    message: &str,
) {
    let Some(message) = v8_string(scope, message) else {
        let _ = resolver.reject(scope, v8::undefined(scope).into());
        return;
    };
    let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
}

pub(super) fn service_worker_push_manager_subscribe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(options) = crate::context_bootstrap::push_interfaces::parse_options(scope, &args)
    else {
        return;
    };
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    if options.has_application_server_key {
        let reason = worker_dom_exception_value(
            scope,
            "Push encryption and application server keys are not supported.",
            "NotSupportedError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }

    if service_worker_push_permission_state(scope) != "granted" {
        let reason = worker_dom_exception_value(
            scope,
            "Push permission has not been granted.",
            "NotAllowedError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'subscribe' on 'PushManager': registration is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'subscribe' on 'PushManager': Service Worker runtime is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let user_visible_only = options.user_visible_only;
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_push_subscribe(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPushSubscribe(
        crate::runtime::ServiceWorkerPushSubscribe {
            request_id,
            registration_id,
            version_id,
            user_visible_only,
        },
    ));
}

pub(super) fn service_worker_push_manager_get_subscription_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'getSubscription' on 'PushManager': registration is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'getSubscription' on 'PushManager': Service Worker runtime is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state
            .register_pending_service_worker_push_get_subscription(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPushGetSubscription(
        crate::runtime::ServiceWorkerPushGetSubscription {
            request_id,
            registration_id,
            version_id,
        },
    ));
}

pub(super) fn service_worker_push_manager_permission_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if crate::context_bootstrap::push_interfaces::parse_options(scope, &args).is_none() {
        return;
    }
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let state = service_worker_push_permission_state(scope);
    let value = v8_string(scope, &state)
        .map(v8::Local::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.resolve(scope, value);
}

fn service_worker_push_permission_state(scope: &mut v8::PinScope<'_, '_>) -> String {
    let Some(state) = get_worker_state(scope) else {
        return "prompt".to_owned();
    };
    let state = state.borrow();
    if !state.secure_context {
        return "denied".to_owned();
    }
    match worker_permission_state(&state, "notifications").as_str() {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

pub(super) fn build_service_worker_push_subscription_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: &ServiceWorkerPushSubscriptionSnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    crate::context_bootstrap::push_interfaces::build_subscription(scope, snapshot)
}

pub(super) fn service_worker_push_subscription_unsubscribe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'unsubscribe' on 'PushSubscription': registration is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'unsubscribe' on 'PushSubscription': Service Worker runtime is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_push_unsubscribe(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPushUnsubscribe(
        crate::runtime::ServiceWorkerPushUnsubscribe {
            request_id,
            registration_id,
            version_id,
        },
    ));
}

pub(super) fn service_worker_skip_waiting_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let undefined: v8::Local<'_, v8::Value> = v8::undefined(scope).into();
        rv.set(resolved_worker_promise(scope, undefined).into());
        return;
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerSkipWaiting {
        registration_id,
        version_id,
    });
    let undefined: v8::Local<'_, v8::Value> = v8::undefined(scope).into();
    rv.set(resolved_worker_promise(scope, undefined).into());
}

pub(super) fn service_worker_clients_claim_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope) {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientsClaim {
            registration_id,
            version_id,
        });
    }
    let undefined: v8::Local<'_, v8::Value> = v8::undefined(scope).into();
    rv.set(resolved_worker_promise(scope, undefined).into());
}

pub(super) fn service_worker_clients_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let id = args.get(0).to_rust_string_lossy(scope);
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_client_query(
            v8::Global::new(scope, resolver),
            PendingServiceWorkerClientQueryType::Get,
        )
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientQuery(
        crate::runtime::ServiceWorkerClientQuery {
            request_id,
            registration_id,
            version_id,
            kind: crate::runtime::ServiceWorkerClientQueryKind::Get {
                exposed_client_id: id,
            },
        },
    ));
}

pub(super) fn service_worker_clients_match_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.resolve(scope, v8::Array::new(scope, 0).into());
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.resolve(scope, v8::Array::new(scope, 0).into());
        return;
    };
    let options = service_worker_client_query_options(scope, args.get(0));
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_client_query(
            v8::Global::new(scope, resolver),
            PendingServiceWorkerClientQueryType::MatchAll,
        )
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientQuery(
        crate::runtime::ServiceWorkerClientQuery {
            request_id,
            registration_id,
            version_id,
            kind: crate::runtime::ServiceWorkerClientQueryKind::MatchAll { options },
        },
    ));
}

pub(super) fn service_worker_clients_open_window_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let url = args.get(0).to_rust_string_lossy(scope);
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let base_url = {
        let state = state.borrow();
        state.current_script_url.clone()
    };
    let parsed_url = base_url
        .as_ref()
        .and_then(|base_url| base_url.join(&url).ok())
        .or_else(|| Url::parse(&url).ok());
    let Some(parsed_url) = parsed_url else {
        let Some(message) = v8_string(scope, &format!("'{url}' is not a valid URL.")) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    if !service_worker_clients_open_window_scheme_can_display(&parsed_url) {
        let Some(message) = v8_string(
            scope,
            &format!("'{}' cannot be opened.", parsed_url.as_str()),
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    }
    if !state
        .borrow_mut()
        .consume_service_worker_window_interaction()
    {
        let reason = worker_dom_exception_value(
            scope,
            "Not allowed to open a window.",
            "InvalidAccessError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }

    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_clients_open_window(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientsOpenWindow(
        crate::runtime::ServiceWorkerClientsOpenWindow {
            request_id,
            source_version_id,
            url: parsed_url,
        },
    ));
}

fn service_worker_clients_open_window_scheme_can_display(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

fn service_worker_client_query_options(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> crate::runtime::ServiceWorkerClientQueryOptions {
    let default = crate::runtime::ServiceWorkerClientQueryOptions {
        include_uncontrolled: false,
        client_type: ServiceWorkerClientQueryType::Window,
    };
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return default;
    };
    let include_uncontrolled = object
        .get(scope, v8str(scope, "includeUncontrolled").into())
        .is_some_and(|value| value.boolean_value(scope));
    let client_type = object
        .get(scope, v8str(scope, "type").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .map(|value| match value.as_str() {
            "all" => ServiceWorkerClientQueryType::All,
            "worker" => ServiceWorkerClientQueryType::Worker,
            "sharedworker" => ServiceWorkerClientQueryType::SharedWorker,
            "window" => ServiceWorkerClientQueryType::Window,
            _ => ServiceWorkerClientQueryType::Window,
        })
        .unwrap_or(ServiceWorkerClientQueryType::Window);
    crate::runtime::ServiceWorkerClientQueryOptions {
        include_uncontrolled,
        client_type,
    }
}

fn resolved_worker_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
    let _ = resolver.resolve(scope, value);
    resolver.get_promise(scope)
}

pub(crate) fn service_worker_runtime_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(
    ServiceWorkerRegistrationId,
    ServiceWorkerVersionId,
    mpsc::UnboundedSender<WorkerToParentMessage>,
)> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let WorkerGlobalKind::Service {
        registration_id,
        version_id,
        ..
    } = &state.global_kind
    else {
        return None;
    };
    Some((*registration_id, *version_id, state.parent_tx.clone()))
}

fn worker_permission_state(state: &WorkerGlobalState, permission_name: &str) -> String {
    let current_origin = state
        .current_script_url
        .as_ref()
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default();
    let mut fallback_state = None;

    for override_entry in state.permission_overrides.iter().rev() {
        let Some(name) = worker_permission_override_name(override_entry) else {
            continue;
        };
        if name != permission_name {
            continue;
        }

        if let Some(embedded_origin) = override_entry.embedded_origin.as_deref() {
            if embedded_origin == current_origin {
                return override_entry.setting.clone();
            }
            continue;
        }

        if override_entry
            .origin
            .as_deref()
            .is_some_and(|origin| origin == current_origin)
        {
            return override_entry.setting.clone();
        }

        if override_entry.origin.is_none() && fallback_state.is_none() {
            fallback_state = Some(override_entry.setting.clone());
        }
    }

    fallback_state.unwrap_or_else(|| match permission_name {
        "background-sync" | "periodic-background-sync" | "persistent-storage" => {
            "granted".to_owned()
        }
        _ => "prompt".to_owned(),
    })
}

pub(crate) fn worker_notification_permission_state(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<String> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    if !state.secure_context {
        return Some("denied".to_owned());
    }
    Some(
        match worker_permission_state(&state, "notifications").as_str() {
            "granted" => "granted",
            "denied" => "denied",
            _ => "default",
        }
        .to_owned(),
    )
}

fn worker_permission_override_name(
    override_entry: &crate::protocol_types::PermissionOverrideRegistration,
) -> Option<&str> {
    match &override_entry.permission {
        serde_json::Value::String(name) => Some(name.as_str()),
        serde_json::Value::Object(map) => map.get("name").and_then(serde_json::Value::as_str),
        _ => None,
    }
}

pub(in crate::worker) fn build_service_worker_client_object_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: &ServiceWorkerClientSnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    build_service_worker_client_object(
        scope,
        client.id,
        &client.exposed_id,
        client.url.as_str(),
        client.client_type.as_webidl_str(),
        client.frame_type.as_webidl_str(),
        client.visibility_state.as_webidl_str(),
        client.focused,
    )
}

pub(in crate::worker) fn build_service_worker_client_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client_id: crate::runtime::ServiceWorkerClientId,
    exposed_client_id: &str,
    url: &str,
    client_type: &'static str,
    frame_type: &'static str,
    visibility_state: &'static str,
    focused: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let url = v8_string(scope, url).unwrap_or_else(|| v8::String::empty(scope));
    let client = if client_type == "window" {
        ServiceWorkerWindowClientDeclaration {
            id: exposed_client_id.to_owned(),
            url,
            client_type,
            frame_type,
            lifecycle_state: "active",
            visibility_state,
            focused,
            post_message: (),
            focus: (),
            navigate: (),
        }
        .bind(scope)
        .ok()?
    } else {
        ServiceWorkerBaseClientDeclaration {
            id: exposed_client_id.to_owned(),
            url,
            client_type,
            post_message: (),
        }
        .bind(scope)
        .ok()?
    };
    set_private_value(
        scope,
        client,
        SERVICE_WORKER_CLIENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, client_id.as_u64()).into(),
    );
    Some(client)
}

fn service_worker_client_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: v8::Local<'s, v8::Object>,
) -> Option<crate::runtime::ServiceWorkerClientId> {
    let value = get_private_value(scope, client, SERVICE_WORKER_CLIENT_ID_SLOT)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, lossless) = big.u64_value();
    lossless.then(|| crate::runtime::ServiceWorkerClientId::from_u64_for_worker(id))
}

fn service_worker_version_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) -> Option<crate::runtime::ServiceWorkerVersionId> {
    let value = get_private_value(scope, worker, SERVICE_WORKER_VERSION_ID_SLOT)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, lossless) = big.u64_value();
    lossless.then(|| crate::runtime::ServiceWorkerVersionId::from_u64_for_binding(id))
}

pub(super) fn service_worker_worker_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'ServiceWorker': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(target_version_id) = service_worker_version_id_from_object(scope, args.this()) else {
        return;
    };
    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        return;
    };
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(payload) = crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        args.get(0),
        transfer_arg,
        "ServiceWorker",
    ) else {
        return;
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerWorkerMessage(
        crate::runtime::ServiceWorkerWorkerMessage {
            source_version_id,
            target_version_id,
            payload,
        },
    ));
}

fn service_worker_runtime_message_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(
    crate::runtime::ServiceWorkerVersionId,
    mpsc::UnboundedSender<WorkerToParentMessage>,
)> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let WorkerGlobalKind::Service { version_id, .. } = &state.global_kind else {
        return None;
    };
    Some((*version_id, state.parent_tx.clone()))
}

pub(super) fn service_worker_client_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'Client': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(target_client_id) = service_worker_client_id_from_object(scope, args.this()) else {
        return;
    };
    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        return;
    };
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(payload) = crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        args.get(0),
        transfer_arg,
        "Client",
    ) else {
        return;
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientMessage(
        crate::runtime::ServiceWorkerClientMessage {
            source_version_id,
            target_client_id,
            payload,
        },
    ));
}

pub(super) fn service_worker_window_client_focus_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    if !state
        .borrow_mut()
        .consume_service_worker_window_interaction()
    {
        let reason = worker_dom_exception_value(
            scope,
            "Not allowed to focus a window.",
            "InvalidAccessError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let Some(target_client_id) = service_worker_client_id_from_object(scope, args.this()) else {
        let Some(message) = v8_string(scope, "The client was not found.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_client_focus(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientFocus(
        crate::runtime::ServiceWorkerClientFocus {
            request_id,
            source_version_id,
            target_client_id,
        },
    ));
}

pub(super) fn service_worker_window_client_navigate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let url = args.get(0).to_rust_string_lossy(scope);
    let Some(target_client_id) = service_worker_client_id_from_object(scope, args.this()) else {
        let Some(message) = v8_string(scope, "The client was not found.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let base_url = {
        let Some(state) = get_worker_state(scope) else {
            let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
            return;
        };
        let state = state.borrow();
        state.current_script_url.clone()
    };
    let parsed_url = base_url
        .as_ref()
        .and_then(|base_url| base_url.join(&url).ok())
        .or_else(|| Url::parse(&url).ok());
    if parsed_url
        .as_ref()
        .is_none_or(|url| url.scheme() == "about")
    {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'navigate' on 'WindowClient': URL is invalid.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    }
    let parsed_url = parsed_url.expect("checked parsed service worker client navigate URL");
    if !service_worker_window_client_can_display_url(&parsed_url) {
        let message = format!("'{}' cannot navigate.", parsed_url.as_str());
        let Some(message) = v8_string(scope, &message) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    }
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_client_navigate(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientNavigate(
        crate::runtime::ServiceWorkerClientNavigate {
            request_id,
            source_version_id,
            target_client_id,
            url: parsed_url,
        },
    ));
}

fn service_worker_window_client_can_display_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}
