//! Worker-owned ServiceWorkerContainer jobs and realm-local registration objects.

use super::*;
use crate::service_worker_runtime::{
    ServiceWorkerClientUrlError, ServiceWorkerRegisterJob, ServiceWorkerRegistrationRequest,
    ServiceWorkerRegistrationSnapshot, ServiceWorkerUpdateViaCache,
    resolve_service_worker_script_url, service_worker_client_url, service_worker_scope_url,
};
use crate::worker::{WorkerNetworkPolicy, WorkerScriptKind};

pub(in crate::worker) struct WorkerServiceWorkerRegistration {
    pub(in crate::worker) snapshot: ServiceWorkerRegistrationSnapshot,
    pub(in crate::worker) object: v8::Global<v8::Object>,
}

fn reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
    dom_name: Option<&str>,
) {
    let error = match dom_name {
        Some(name) => crate::context_bootstrap::new_dom_exception_value(scope, message, name),
        None => {
            let message = v8_string(scope, message)
                .unwrap_or_else(|| v8str(scope, "ServiceWorker operation failed"));
            v8::Exception::type_error(scope, message)
        }
    };
    let _ = resolver.reject(scope, error);
}

pub(crate) fn try_worker_service_worker_register<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    script_argument: v8::Local<'s, v8::Value>,
    requested_scope: Option<&str>,
    script_kind: WorkerScriptKind,
    update_via_cache: ServiceWorkerUpdateViaCache,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let requirements = worker_trusted_types_for_script_requirements(scope).unwrap_or_default();
    let Some(script) = crate::context_bootstrap::trusted_script_url_string_or_throw(
        scope,
        script_argument,
        requirements,
        "ServiceWorkerContainer register",
        "register",
    ) else {
        return true;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return true;
    };
    rv.set(resolver.get_promise(scope).into());
    let request = {
        let mut state = state.borrow_mut();
        let Some(document_url) = state.current_script_url.clone() else {
            reject(
                scope,
                resolver,
                "The worker settings are unavailable.",
                Some("InvalidStateError"),
            );
            return true;
        };
        if !state.secure_context || state.storage_key.origin() == "null" {
            reject(
                scope,
                resolver,
                "The worker origin cannot register service workers.",
                Some("SecurityError"),
            );
            return true;
        }
        let Some(script_url) = resolve_service_worker_script_url(&document_url, &script) else {
            reject(
                scope,
                resolver,
                "Failed to parse the service worker script URL.",
                None,
            );
            return true;
        };
        let Some(scope_url) = service_worker_scope_url(&document_url, &script_url, requested_scope)
        else {
            reject(
                scope,
                resolver,
                "Failed to parse the service worker scope URL.",
                None,
            );
            return true;
        };
        if let Err(message) = crate::service_worker_runtime::verify_service_worker_registration_urls(
            &scope_url,
            &script_url,
        ) {
            reject(scope, resolver, &message, None);
            return true;
        }
        if !moli_url::same_origin(&document_url, &script_url)
            || !moli_url::same_origin(&document_url, &scope_url)
        {
            reject(
                scope,
                resolver,
                "The script and scope must have the worker's origin.",
                Some("SecurityError"),
            );
            return true;
        }
        let Some(runtime) = state.service_worker_runtime.clone() else {
            reject(
                scope,
                resolver,
                "The ServiceWorker runtime is unavailable.",
                Some("InvalidStateError"),
            );
            return true;
        };
        let storage_key = state.storage_key.serialized_storage_key();
        // Subscribe before starting the job. The owner queues the installing
        // snapshot, promise result, and updatefound as separate worker tasks.
        runtime.watch_registration_from_worker_client(
            &scope_url,
            &storage_key,
            &state.worker_wake_tx,
        );
        let request_id = state.register_pending_service_worker_update(PendingServiceWorkerUpdate {
            resolver: v8::Global::new(scope, resolver),
            registration: None,
        });
        let request = ServiceWorkerRegistrationRequest {
            script_url,
            scope_url,
            document_url,
            storage_key,
            script_kind,
            update_via_cache,
            request_client: state.loader.request_client().clone(),
            network_policy: WorkerNetworkPolicy {
                secure_context: state.secure_context,
                permission_overrides: state.permission_overrides.clone(),
                extra_http_headers: state.extra_http_headers.clone(),
                network_offline: state.network_offline,
                blocked_url_patterns: state.blocked_url_patterns.clone(),
                network_partition_key: state.network_partition_key.clone(),
                fetch_subresource_interception_enabled: state
                    .fetch_subresource_interception_enabled,
                fetch_subresource_interception_resource_type: state
                    .fetch_subresource_interception_resource_type,
            },
            worker_context_runtime: state.worker_context_runtime.clone(),
            broadcast_channel_top_level_site: Some(state.storage_key.top_level_site().to_owned()),
            indexed_db_manager: state.indexed_db_manager.clone(),
            storage_bucket_store: state.storage_bucket_store.clone(),
            completion: ServiceWorkerRegisterJob::Worker {
                request_id,
                completion_tx: state.worker_wake_tx.clone(),
            },
        };
        (runtime, request)
    };
    request.0.start_registration_job(request.1);
    true
}

pub(crate) fn try_worker_service_worker_get_registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: &str,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let snapshot = {
        let state = state.borrow();
        let Some(base) = state.current_script_url.as_ref() else {
            reject(
                scope,
                resolver,
                "The worker settings are unavailable.",
                Some("InvalidStateError"),
            );
            return true;
        };
        let origin = worker_global_origin(scope).unwrap_or(moli_url::WebOrigin::Opaque);
        let url = match service_worker_client_url(base, &origin, client) {
            Ok(url) => url,
            Err(ServiceWorkerClientUrlError::Invalid) => {
                reject(scope, resolver, "Failed to parse the client URL.", None);
                return true;
            }
            Err(ServiceWorkerClientUrlError::CrossOrigin) => {
                reject(
                    scope,
                    resolver,
                    "The client URL must have the worker's origin.",
                    Some("SecurityError"),
                );
                return true;
            }
        };
        state.service_worker_runtime.as_ref().and_then(|runtime| {
            let storage_key = state.storage_key.serialized_storage_key();
            let snapshot =
                runtime.matching_registration_for_client_with_storage_key(&url, &storage_key)?;
            runtime.watch_registration_from_worker_client(
                snapshot.scope_url(),
                &storage_key,
                &state.worker_wake_tx,
            );
            Some(snapshot)
        })
    };
    let value = snapshot
        .and_then(|snapshot| build_worker_service_worker_registration(scope, &snapshot).ok())
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.resolve(scope, value);
    true
}

pub(crate) fn try_worker_service_worker_get_registrations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let snapshots = {
        let state = state.borrow();
        state
            .service_worker_runtime
            .as_ref()
            .zip(state.current_script_url.as_ref())
            .map(|(runtime, url)| {
                let storage_key = state.storage_key.serialized_storage_key();
                let snapshots = runtime.all_registrations_with_storage_key(url, &storage_key);
                for snapshot in &snapshots {
                    runtime.watch_registration_from_worker_client(
                        snapshot.scope_url(),
                        &storage_key,
                        &state.worker_wake_tx,
                    );
                }
                snapshots
            })
            .unwrap_or_default()
    };
    let result = v8::Array::new(scope, snapshots.len() as i32);
    for (index, snapshot) in snapshots.iter().enumerate() {
        if let Ok(object) = build_worker_service_worker_registration(scope, snapshot) {
            let _ = result.set_index(scope, index as u32, object.into());
        }
    }
    let _ = result.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    let _ = resolver.resolve(scope, result.into());
    true
}

pub(super) fn build_worker_service_worker_registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: &ServiceWorkerRegistrationSnapshot,
) -> Result<v8::Local<'s, v8::Object>> {
    let state =
        get_worker_state(scope).ok_or_else(|| anyhow!("Worker settings are unavailable"))?;
    if let Some(entry) = state
        .borrow()
        .service_worker_registrations
        .get(&snapshot.registration_id())
    {
        return Ok(v8::Local::new(scope, &entry.object));
    }
    let own_registration = matches!(state.borrow().global_kind, WorkerGlobalKind::Service { registration_id, .. } if registration_id == snapshot.registration_id());
    let global = scope.get_current_context().global(scope);
    let object = if own_registration {
        get_private_value(scope, global, SERVICE_WORKER_GLOBAL_REGISTRATION_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    } else {
        None
    };
    let object = match object {
        Some(object) => object,
        None => build_service_worker_global_registration(
            scope,
            snapshot.registration_id(),
            snapshot
                .active()
                .or_else(|| snapshot.waiting())
                .or_else(|| snapshot.installing())
                .map(|version| version.version_id()),
            snapshot.scope_url(),
        )?,
    };
    set_private_value(
        scope,
        object,
        crate::context_bootstrap::service_worker_interfaces::REGISTRATION_UPDATE_VIA_CACHE,
        v8str(scope, snapshot.update_via_cache().as_str()).into(),
    );
    state.borrow_mut().service_worker_registrations.insert(
        snapshot.registration_id(),
        WorkerServiceWorkerRegistration {
            snapshot: snapshot.clone(),
            object: v8::Global::new(scope, object),
        },
    );
    Ok(object)
}

pub(super) fn worker_service_worker_registration_unregister<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let scope_url = get_private_value(scope, registration, SERVICE_WORKER_REGISTRATION_SCOPE_SLOT)
        .and_then(|value| value.to_string(scope))
        .and_then(|value| Url::parse(&value.to_rust_string_lossy(scope)).ok());
    let Some(scope_url) = scope_url else {
        reject(
            scope,
            resolver,
            "The registration is unavailable.",
            Some("InvalidStateError"),
        );
        return;
    };
    let (runtime, storage_key, request_id, completion_tx) = {
        let mut state = state.borrow_mut();
        let Some(runtime) = state.service_worker_runtime.clone() else {
            reject(
                scope,
                resolver,
                "The ServiceWorker runtime is unavailable.",
                Some("InvalidStateError"),
            );
            return;
        };
        let request_id =
            state.register_pending_service_worker_unregister(v8::Global::new(scope, resolver));
        (
            runtime,
            state.storage_key.serialized_storage_key(),
            request_id,
            state.worker_wake_tx.clone(),
        )
    };
    runtime.start_unregistration_from_worker(&scope_url, storage_key, request_id, completion_tx);
}
