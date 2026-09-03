use super::*;

struct WorkerOptionsMembers {
    worker_type: WorkerScriptKind,
    name: String,
    credentials_mode: moli_fetch::RequestCredentialsMode,
}

use crate::context_bootstrap::navigation_serialize::{
    current_document_content_security_policies, current_document_referrer_policy,
};
use crate::document_runtime::DomHandle;
use crate::service_worker_runtime::{ServiceWorkerClientId, ServiceWorkerClientType};
use crate::types::DedicatedWorkerId;
use crate::webidl;
use crate::worker::{NestedWorkerContext, WorkerScriptKind};
use moli_encoding::{encode_url_query_for_legacy_web, form_output_encoding_for_label};
use moli_v8_util::v8_string;
use moli_webapi_declare::WebApiObject;
use url::Url;

const WORKER_INVALID_URL_MESSAGE: &str =
    "Failed to construct 'Worker': the provided URL is invalid.";
const WORKER_SCRIPT_URL_CONVERSION_SLOT: &str = "__moliWorkerScriptUrlConversion";
const WORKER_RECURSIVE_CONSTRUCTOR_MESSAGE: &str = "Maximum call stack size exceeded";

#[derive(Copy, Clone)]
struct WorkerEventHandler {
    slot_name: &'static str,
    event_type: &'static str,
}

const WORKER_EVENT_HANDLERS: &[WorkerEventHandler] = &[
    WorkerEventHandler {
        slot_name: WORKER_ONMESSAGE_SLOT,
        event_type: "message",
    },
    WorkerEventHandler {
        slot_name: WORKER_ONMESSAGEERROR_SLOT,
        event_type: "messageerror",
    },
    WorkerEventHandler {
        slot_name: WORKER_ONERROR_SLOT,
        event_type: "error",
    },
];

#[derive(WebApiObject)]
#[webapi(interface = "Worker")]
struct WorkerObjectDeclaration {
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = WORKER_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(slot = WORKER_ONMESSAGE_SLOT, init = "null")]
    onmessage_slot: (),

    #[webapi(slot = WORKER_ONMESSAGEERROR_SLOT, init = "null")]
    onmessageerror_slot: (),

    #[webapi(slot = WORKER_ONERROR_SLOT, init = "null")]
    onerror_slot: (),

    #[webapi(method, enumerable, callback = simple_event_target_add_event_listener_callback)]
    add_event_listener: (),

    #[webapi(
        method,
        enumerable,
        callback = simple_event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),

    #[webapi(method, enumerable, callback = simple_event_target_dispatch_event_callback)]
    dispatch_event: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = worker_event_handler_getter,
        setter = worker_event_handler_setter,
        data = callback_data_index_value(scope, 0)
    )]
    onmessage: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = worker_event_handler_getter,
        setter = worker_event_handler_setter,
        data = callback_data_index_value(scope, 1)
    )]
    onmessageerror: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = worker_event_handler_getter,
        setter = worker_event_handler_setter,
        data = callback_data_index_value(scope, 2)
    )]
    onerror: (),
}

pub(in crate::context_bootstrap) fn worker_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Worker': Please use the 'new' operator.",
        );
        return;
    }

    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to construct 'Worker': 1 argument required, but only 0 present.",
        );
        return;
    }
    let script_url_value = args.get(0);
    let worker_options = match parse_worker_options(scope, &args) {
        Ok(worker_options) => worker_options,
        Err(error) => {
            throw_type_error(scope, &error);
            return;
        }
    };
    let worker = args.this();
    WorkerObjectDeclaration::new()
        .initialize(scope, worker)
        .expect("Worker declaration should initialize");
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let requirements = unsafe { &*host_ptr }.trusted_types_for_script_requirements(scope);
        let Some(script_url_input) = trusted_worker_script_url_string_or_throw(
            scope,
            script_url_value,
            requirements,
            "Worker constructor",
            "Worker",
        ) else {
            return;
        };
        let global = scope.get_current_context().global(scope);
        let document_meta_content_security_policies =
            current_document_content_security_policies(scope, global);
        let document_referrer_policy = current_document_referrer_policy(scope, global);
        let host = unsafe { &mut *host_ptr };
        let child_handle = current_child_context_handle(scope)
            .or_else(|| child_context_handle_from_object_creation_context(scope, worker))
            .or_else(|| {
                host.active_child_subresource_request_scope()
                    .map(|(handle, _, _)| handle)
            })
            .or_else(|| crate::native_bridge::active_child_window_handle(scope));
        let active_popup_base_url = child_handle
            .is_none()
            .then(|| host.active_lightweight_popup_base_url(scope))
            .flatten();
        let base_url = child_handle
            .and_then(|handle| host.child_browsing_context_base_url(handle))
            .or(active_popup_base_url)
            .unwrap_or_else(|| worker_constructor_base_url(host));
        let network_partition_key = child_handle
            .and_then(|handle| host.child_browsing_context_network_partition_key(handle));
        let query_encoding = document_query_encoding_override(host);
        let creator_storage_key = host
            .active_storage_context(scope, child_handle)
            .storage_key()
            .clone();
        let creator_top_level_site = creator_storage_key.top_level_site().to_owned();
        let owner_scope = if let Some(handle) = child_handle {
            crate::native_bridge::WorkerOwnerScope::Child(handle)
        } else if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope) {
            crate::native_bridge::WorkerOwnerScope::LightweightPopup(popup_id)
        } else {
            crate::native_bridge::WorkerOwnerScope::Top
        };
        let dispatch_scope = crate::native_bridge::OwnerDispatchScope::from(owner_scope);
        let mut document_content_security_policies = if let Some(handle) = child_handle {
            host.child_browsing_context_content_security_policies(handle)
                .map(<[String]>::to_vec)
                .unwrap_or_else(|| host.document_content_security_policies().to_vec())
        } else if let Some(policies) =
            host.active_lightweight_popup_content_security_policies(scope)
        {
            policies.to_vec()
        } else {
            host.document_content_security_policies().to_vec()
        };
        document_content_security_policies.extend(document_meta_content_security_policies);
        let Some(creator_document_loader) =
            host.document_resource_loader_for_dispatch_scope(dispatch_scope)
        else {
            throw_type_error(
                scope,
                "Failed to construct 'Worker': execution context is no longer available.",
            );
            return;
        };
        let creator_request_client = creator_document_loader.request_client().clone();
        let Some(execution_context_owner) =
            host.current_window_execution_context_binding(scope, dispatch_scope)
        else {
            throw_type_error(
                scope,
                "Failed to construct 'Worker': execution context is no longer available.",
            );
            return;
        };
        let creator_policy_context =
            crate::network_host::effective_subresource_policy_context(scope, host, dispatch_scope);
        let resolved_url =
            match resolve_worker_script_url(&base_url, &script_url_input, query_encoding) {
                Ok(url) => url,
                Err(message) => {
                    throw_worker_dom_exception(scope, "SyntaxError", 12, &message);
                    return;
                }
            };
        if !worker_script_scheme_can_load(&resolved_url) {
            let Some(outside_settings_load) =
                host.register_dedicated_worker_outside_settings_load(dispatch_scope)
            else {
                throw_type_error(
                    scope,
                    "Failed to construct 'Worker': execution context is no longer available.",
                );
                return;
            };
            let worker_id = host.register_loading_worker(
                scope,
                worker,
                creator_top_level_site,
                creator_storage_key,
                worker_options.name.clone(),
                worker_options.credentials_mode,
                None,
                outside_settings_load,
                execution_context_owner,
            );
            let recorded = host.record_dedicated_worker_target_created(
                worker_id,
                base_url,
                worker_script_resource_url(&resolved_url),
                worker_options.name,
            );
            debug_assert!(
                recorded,
                "newly registered DedicatedWorker must publish its target before load failure"
            );
            let started = host.start_failed_worker_script_load(
                worker_id,
                resolved_url,
                "Failed to load worker script: unsupported worker script URL scheme.",
            );
            if !started {
                host.forget_worker(worker_id);
                throw_type_error(
                    scope,
                    "Failed to construct 'Worker': runtime is unavailable.",
                );
                return;
            }
            set_private_value(
                scope,
                worker,
                WORKER_ID_SLOT,
                v8::Number::new(scope, worker_id.as_u64() as f64).into(),
            );
            rv.set(worker.into());
            return;
        }
        let materialized = match materialize_worker_script_source(&resolved_url) {
            Ok(source) => source,
            Err(message) => {
                throw_type_error(scope, &message);
                return;
            }
        };
        let creator_secure_context = moli_url::is_potentially_trustworthy_url(&base_url);
        let worker_id = if let Some(script_source) = materialized {
            let request_url = worker_script_resource_url(&resolved_url);
            let network_response =
                local_worker_main_script_network_response(&resolved_url, &script_source);
            let secure_context =
                worker_secure_context_for_script_url(&resolved_url, creator_secure_context);
            let network_policy = WorkerNetworkPolicy {
                secure_context,
                permission_overrides: host.permission_overrides().to_vec(),
                extra_http_headers: host.extra_http_headers().to_vec(),
                network_offline: host.network_offline(),
                blocked_url_patterns: host.blocked_url_patterns().to_vec(),
                network_partition_key: network_partition_key.clone(),
                fetch_subresource_interception_enabled: host
                    .fetch_subresource_interception_enabled(),
                fetch_subresource_interception_resource_type: host
                    .fetch_subresource_interception_resource_type(),
            };
            let parent_service_worker_client_id =
                host.service_worker_client_id_for_worker_owner(owner_scope);
            let reserved_service_worker_client_id =
                reserve_inherited_service_worker_worker_client_for_local_script(
                    host.browser_context_runtime(),
                    &resolved_url,
                    creator_storage_key.serialized_storage_key(),
                    secure_context,
                    parent_service_worker_client_id,
                );
            let mut worker_policy_context = creator_policy_context;
            if resolved_url.scheme() == "data" {
                worker_policy_context.cross_origin_isolated = false;
            }
            let mut options = WorkerSpawnOptions::new_with_request_client(
                script_source,
                resolved_url.to_string(),
                creator_request_client.clone(),
            )
            .with_script_kind(worker_options.worker_type)
            .with_module_credentials_mode(worker_options.credentials_mode)
            .with_module_static_import_initiator_url(base_url.clone())
            .with_module_static_import_content_security_policies(document_content_security_policies)
            .with_network_policy(network_policy)
            .with_policy_context(worker_policy_context)
            .with_worker_context_runtime(host.browser_context_runtime().worker_context_runtime())
            .with_service_worker_runtime(host.browser_context_runtime().service_worker_runtime())
            .with_global_kind(WorkerGlobalKind::Dedicated {
                name: worker_options.name.clone(),
            })
            .with_storage_key_top_level_site(Some(creator_top_level_site.clone()))
            .with_creator_storage_key(creator_storage_key)
            .with_indexed_db_manager(host.indexed_db_manager())
            .with_storage_bucket_store(Some(host.storage_bucket_store()))
            .with_pause_evaluation_until_debugger(
                host.browser_context_runtime()
                    .dedicated_worker_pause_on_start_for_devtools(),
            );
            if let Some(client_id) = reserved_service_worker_client_id {
                options = options.with_reserved_service_worker_client_id(client_id);
            }
            let worker_handle = spawn_worker_with_options(options);
            let worker_id =
                host.register_worker(scope, worker, worker_handle, execution_context_owner);
            let recorded = host.record_dedicated_worker_target_created(
                worker_id,
                base_url.clone(),
                request_url,
                worker_options.name.clone(),
            );
            debug_assert!(
                recorded,
                "newly registered local DedicatedWorker must publish its target"
            );
            let recorded = host.record_dedicated_worker_target_script_loaded(
                worker_id,
                resolved_url.to_string(),
                Box::new(network_response),
            );
            debug_assert!(
                recorded,
                "newly registered local DedicatedWorker must publish its main script response"
            );
            worker_id
        } else {
            let cross_origin_http_worker_script =
                is_cross_origin_http_worker_script(&base_url, &resolved_url);
            let reserved_service_worker_client_id = if cross_origin_http_worker_script {
                None
            } else {
                Some(
                    host.browser_context_runtime()
                        .register_reserved_service_worker_worker_client(
                            resolved_url.clone(),
                            creator_storage_key.serialized_storage_key(),
                            ServiceWorkerClientType::DedicatedWorker,
                            worker_secure_context_for_script_url(
                                &resolved_url,
                                creator_secure_context,
                            ),
                        ),
                )
            };
            let Some(outside_settings_load) =
                host.register_dedicated_worker_outside_settings_load(dispatch_scope)
            else {
                throw_type_error(
                    scope,
                    "Failed to construct 'Worker': execution context is no longer available.",
                );
                return;
            };
            let worker_id = host.register_loading_worker(
                scope,
                worker,
                creator_top_level_site,
                creator_storage_key,
                worker_options.name.clone(),
                worker_options.credentials_mode,
                reserved_service_worker_client_id,
                outside_settings_load,
                execution_context_owner,
            );
            let recorded = host.record_dedicated_worker_target_created(
                worker_id,
                base_url.clone(),
                worker_script_resource_url(&resolved_url),
                worker_options.name.clone(),
            );
            debug_assert!(
                recorded,
                "newly registered external DedicatedWorker must publish its target before loading"
            );
            let started = if cross_origin_http_worker_script {
                host.start_failed_worker_script_load(
                    worker_id,
                    resolved_url,
                    "Failed to load worker script: cross-origin worker script blocked.",
                )
            } else {
                host.start_worker_script_load(
                    worker_id,
                    resolved_url,
                    base_url,
                    network_partition_key,
                    creator_policy_context,
                    worker_options.worker_type,
                    worker_options.credentials_mode,
                    document_referrer_policy,
                    document_content_security_policies,
                    worker_options.name.clone(),
                    reserved_service_worker_client_id,
                )
            };
            if !started {
                host.forget_worker(worker_id);
                throw_type_error(
                    scope,
                    "Failed to construct 'Worker': runtime is unavailable.",
                );
                return;
            }
            worker_id
        };
        set_private_value(
            scope,
            worker,
            WORKER_ID_SLOT,
            v8::Number::new(scope, worker_id.as_u64() as f64).into(),
        );
    } else if let Some(nested_context) = crate::worker::reserve_nested_worker_context(scope, worker)
    {
        let Some(script_url_input) = trusted_worker_script_url_string_or_throw(
            scope,
            script_url_value,
            crate::content_security_policy::TrustedTypesForScriptRequirements::enforced_only(
                nested_context.require_trusted_types_for_script,
            ),
            "Worker constructor",
            "Worker",
        ) else {
            return;
        };
        let resolved_url =
            match resolve_worker_script_url(&nested_context.base_url, &script_url_input, None) {
                Ok(url) => url,
                Err(message) => {
                    throw_worker_dom_exception(scope, "SyntaxError", 12, &message);
                    return;
                }
            };
        if let Err(message) =
            crate::worker::check_and_queue_nested_worker_constructor_csp(scope, &resolved_url)
        {
            queue_nested_worker_script_load_error(
                scope,
                worker,
                &nested_context,
                &resolved_url,
                message,
            );
            return;
        }
        let (script_url, script_source) =
            match materialize_nested_worker_script_source(&resolved_url, &nested_context) {
                Ok(source) => source,
                Err(message) => {
                    queue_nested_worker_script_load_error(
                        scope,
                        worker,
                        &nested_context,
                        &resolved_url,
                        message,
                    );
                    return;
                }
            };
        let mut network_policy = nested_context.network_policy.clone();
        network_policy.secure_context = Url::parse(&script_url).ok().is_some_and(|script_url| {
            worker_secure_context_for_script_url(
                &script_url,
                nested_context.network_policy.secure_context,
            )
        });
        let reserved_service_worker_client_id =
            reserve_nested_inherited_service_worker_worker_client_for_local_script(
                &resolved_url,
                &nested_context,
                network_policy.secure_context,
            );
        let mut worker_policy_context = nested_context.policy_context;
        if resolved_url.scheme() == "data" {
            worker_policy_context.cross_origin_isolated = false;
        }
        let options = WorkerSpawnOptions::new_with_request_client(
            script_source,
            script_url,
            nested_context.loader.request_client().clone(),
        )
        .with_script_kind(worker_options.worker_type)
        .with_module_credentials_mode(worker_options.credentials_mode)
        .with_module_static_import_initiator_url(nested_context.base_url.clone())
        .with_module_static_import_content_security_policies(
            nested_context.module_static_import_content_security_policies,
        )
        .with_network_policy(network_policy)
        .with_policy_context(worker_policy_context)
        .with_worker_context_runtime(nested_context.worker_context_runtime.clone())
        .with_global_kind(WorkerGlobalKind::Dedicated {
            name: worker_options.name.clone(),
        })
        .with_storage_key_top_level_site(Some(nested_context.storage_key_top_level_site.clone()))
        .with_creator_storage_key(nested_context.creator_storage_key.clone())
        .with_indexed_db_manager(nested_context.indexed_db_manager.clone())
        .with_storage_bucket_store(nested_context.storage_bucket_store.clone());
        let options = if let Some(runtime) = nested_context.service_worker_runtime.clone() {
            options.with_service_worker_runtime(runtime)
        } else {
            options
        };
        let options = if let Some(client_id) = reserved_service_worker_client_id {
            options.with_reserved_service_worker_client_id(client_id)
        } else {
            options
        };
        let mut worker_handle = spawn_worker_with_options(options);
        if let Some(mut rx) = worker_handle.take_receiver() {
            let wake_tx = nested_context.wake_tx.clone();
            let worker_id = nested_context.worker_id;
            let _ = std::thread::Builder::new()
                .name(format!("nested-worker-pump:{worker_id}"))
                .spawn(move || {
                    while let Some(message) = rx.blocking_recv() {
                        let _ = wake_tx.send(crate::worker::WorkerMessage::NestedWorkerEvent {
                            worker_id,
                            message: Box::new(message),
                        });
                    }
                });
        }
        let handle_box = Box::new(worker_handle);
        let handle_ptr = Box::into_raw(handle_box);
        let external = v8::External::new(scope, handle_ptr as *mut std::ffi::c_void);
        set_private_value(scope, worker, WORKER_HANDLE_SLOT, external.into());
        set_private_value(
            scope,
            worker,
            WORKER_ID_SLOT,
            v8::Number::new(scope, nested_context.worker_id.as_u64() as f64).into(),
        );
    } else {
        #[cfg(not(test))]
        {
            throw_type_error(
                scope,
                "Failed to construct 'Worker': execution context is no longer available.",
            );
            return;
        }

        #[cfg(test)]
        {
            // Narrow V8 surface tests intentionally install only the Worker
            // constructor. Their isolate owns one standalone fetch runtime;
            // each Worker receives only its cloneable request handle. The
            // production callback never admits a Worker without a registered
            // Document or parent Worker authority.
            let Some(request_client) = scope
                .get_slot::<crate::network::ResourceRequestClientOwner>()
                .map(crate::network::ResourceRequestClientOwner::handle)
            else {
                throw_type_error(
                    scope,
                    "Failed to construct 'Worker': standalone test fetch runtime is unavailable.",
                );
                return;
            };
            let Some(script_url_input) = trusted_worker_script_url_string_or_throw(
                scope,
                script_url_value,
                crate::content_security_policy::TrustedTypesForScriptRequirements::default(),
                "Worker constructor",
                "Worker",
            ) else {
                return;
            };
            let script_source = match Url::parse(&script_url_input)
                .ok()
                .map(|url| materialize_worker_script_source(&url))
            {
                Some(Ok(Some(source))) => source,
                Some(Ok(None)) | None => script_url_input.clone(),
                Some(Err(message)) => {
                    throw_type_error(scope, &message);
                    return;
                }
            };
            let worker_handle = spawn_worker_with_options(
                WorkerSpawnOptions::new_with_request_client(
                    script_source,
                    script_url_input,
                    request_client,
                )
                .with_script_kind(worker_options.worker_type),
            );
            let handle_box = Box::new(worker_handle);
            let handle_ptr = Box::into_raw(handle_box);
            let external = v8::External::new(scope, handle_ptr as *mut std::ffi::c_void);
            set_private_value(scope, worker, WORKER_HANDLE_SLOT, external.into());
        }
    }

    rv.set(worker.into());
}

pub(in crate::context_bootstrap) fn trusted_worker_script_url_string_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    requirements: crate::content_security_policy::TrustedTypesForScriptRequirements,
    sink: &'static str,
    api_name: &'static str,
) -> Option<String> {
    let global = scope.get_current_context().global(scope);
    let previous = get_private_value(scope, global, WORKER_SCRIPT_URL_CONVERSION_SLOT);
    if previous.as_ref().is_some_and(|value| value.is_true()) {
        throw_range_error(scope, WORKER_RECURSIVE_CONSTRUCTOR_MESSAGE);
        return None;
    }

    set_private_value(
        scope,
        global,
        WORKER_SCRIPT_URL_CONVERSION_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let result = crate::context_bootstrap::trusted_script_url_string_or_throw(
        scope,
        value,
        requirements,
        sink,
        api_name,
    );
    let global = scope.get_current_context().global(scope);
    let restored = previous.unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, global, WORKER_SCRIPT_URL_CONVERSION_SLOT, restored);
    result
}

fn worker_event_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler) =
        callback_data_item(scope, &args, WORKER_EVENT_HANDLERS, "Worker event handlers")
    else {
        return;
    };
    let value = get_private_value(scope, args.this(), handler.slot_name)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

fn worker_event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(handler) =
        callback_data_item(scope, &args, WORKER_EVENT_HANDLERS, "Worker event handlers")
    else {
        return;
    };
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, args.this(), handler.slot_name, stored);
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        WORKER_LISTENERS_SLOT,
        handler.event_type,
        handler.slot_name,
        stored.is_function(),
    );
    if handler.event_type == "message" && stored.is_function() {
        super::flush_pending_worker_messages_for_listener(scope, args.this());
    }
}

fn parse_worker_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<WorkerOptionsMembers, String> {
    let options = webidl::dictionary_arg(args, 1, webidl::Context::argument("WorkerOptions", 2))
        .map_err(|_| {
            "Failed to construct 'Worker': the provided options are invalid.".to_owned()
        })?;
    let Some(options) = options else {
        return Ok(WorkerOptionsMembers {
            worker_type: WorkerScriptKind::Classic,
            name: String::new(),
            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
        });
    };
    let worker_type = webidl::optional_member_or::<webidl::EnumValue<WorkerScriptKind>>(
        scope,
        options,
        "type",
        webidl::Context::member("WorkerOptions", "type"),
        webidl::EnumValue(WorkerScriptKind::Classic),
    )
    .map(|value| value.0)
    .map_err(|_| "Failed to construct 'Worker': the provided worker type is invalid.".to_owned())?;
    let name = webidl::optional_member_or::<webidl::DomString>(
        scope,
        options,
        "name",
        webidl::Context::member("WorkerOptions", "name"),
        webidl::DomString(String::new()),
    )
    .map(|value| value.0)
    .map_err(|_| {
        "Failed to construct 'Worker': the provided worker options are invalid.".to_owned()
    })?;
    let credentials_mode = worker_options_credentials_mode(scope, options)?;
    Ok(WorkerOptionsMembers {
        worker_type,
        name,
        credentials_mode,
    })
}

fn worker_options_credentials_mode(
    scope: &mut v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Object>,
) -> Result<moli_fetch::RequestCredentialsMode, String> {
    let key = v8_string(scope, "credentials").ok_or_else(|| {
        "Failed to construct 'Worker': the provided worker options are invalid.".to_owned()
    })?;
    let Some(value) = options.get(scope, key.into()) else {
        return Err(
            "Failed to construct 'Worker': the provided worker options are invalid.".to_owned(),
        );
    };
    if value.is_undefined() {
        return Ok(moli_fetch::RequestCredentialsMode::SameOrigin);
    }
    let Some(value) = value.to_string(scope) else {
        return Err(
            "Failed to construct 'Worker': the provided worker options are invalid.".to_owned(),
        );
    };
    match value.to_rust_string_lossy(scope).as_str() {
        "omit" => Ok(moli_fetch::RequestCredentialsMode::Omit),
        "same-origin" => Ok(moli_fetch::RequestCredentialsMode::SameOrigin),
        "include" => Ok(moli_fetch::RequestCredentialsMode::Include),
        _ => Err(
            "Failed to construct 'Worker': the provided credentials mode is invalid.".to_owned(),
        ),
    }
}

pub(in crate::context_bootstrap) fn resolve_worker_script_url(
    base_url: &Url,
    input: &str,
    query_encoding: Option<&'static encoding_rs::Encoding>,
) -> Result<Url, String> {
    if let Some(encoding) = query_encoding {
        let input = encode_url_query_for_legacy_web(input, encoding);
        return Url::options()
            .base_url(Some(base_url))
            .parse(input.as_ref())
            .map_err(|_| WORKER_INVALID_URL_MESSAGE.to_owned());
    }
    Url::options()
        .base_url(Some(base_url))
        .parse(input)
        .map_err(|_| WORKER_INVALID_URL_MESSAGE.to_owned())
}

pub(in crate::context_bootstrap) fn document_query_encoding_override(
    host: &crate::native_bridge::JsContextHost,
) -> Option<&'static encoding_rs::Encoding> {
    form_output_encoding_for_label(host.document_character_set())
        .filter(|encoding| *encoding != encoding_rs::UTF_8)
}

fn queue_nested_worker_script_load_error(
    scope: &mut v8::PinScope<'_, '_>,
    worker: v8::Local<'_, v8::Object>,
    context: &NestedWorkerContext,
    script_url: &Url,
    message: String,
) {
    let _ = context
        .wake_tx
        .send(crate::worker::WorkerMessage::NestedWorkerEvent {
            worker_id: context.worker_id,
            message: Box::new(crate::worker::WorkerToParentMessage::Error {
                message,
                filename: script_url.to_string(),
                lineno: 0,
                colno: 0,
                event_kind: crate::worker::WorkerParentErrorEventKind::Event,
                phase: crate::worker::WorkerErrorPhase::Runtime,
                source: crate::worker::WorkerErrorSource::Runtime,
            }),
        });
    set_private_value(
        scope,
        worker,
        WORKER_ID_SLOT,
        v8::Number::new(scope, context.worker_id.as_u64() as f64).into(),
    );
}

pub(in crate::context_bootstrap) fn worker_constructor_base_url(
    host: &crate::native_bridge::JsContextHost,
) -> Url {
    host.active_child_subresource_request_scope()
        .map(|(_, _, document_url)| document_url)
        .unwrap_or_else(|| {
            host.dom_host()
                .document_base_url()
                .unwrap_or_else(|| host.document_url().clone())
        })
}

pub(in crate::context_bootstrap) fn worker_script_resource_url(script_url: &Url) -> Url {
    let mut url = script_url.clone();
    url.set_fragment(None);
    url
}

pub(in crate::context_bootstrap) fn worker_script_scheme_can_load(script_url: &Url) -> bool {
    matches!(script_url.scheme(), "http" | "https" | "data" | "blob")
}

fn current_child_context_handle(scope: &mut v8::PinScope<'_, '_>) -> Option<DomHandle> {
    let global = scope.get_current_context().global(scope);
    child_context_handle_from_global(scope, global)
}

fn child_context_handle_from_object_creation_context(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<DomHandle> {
    let context = object.get_creation_context(scope)?;
    let global = context.global(scope);
    child_context_handle_from_global(scope, global)
}

fn child_context_handle_from_global<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let value = get_private_value(
        scope,
        global,
        crate::context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
    )?;
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| DomHandle::new(value as usize))
}

fn materialize_nested_worker_script_source(
    script_url: &Url,
    context: &NestedWorkerContext,
) -> Result<(String, String), String> {
    if let Some(source) = materialize_worker_script_source(script_url)? {
        return Ok((script_url.to_string(), source));
    }
    let loader = context.loader.clone();
    let resource_url = worker_script_resource_url(script_url);
    let request = moli_fetch::Request::new("GET", resource_url.as_str(), None, vec![])
        .map_err(|error| error.to_string())?
        .with_page_network_policy()
        .with_network_partition_key(context.network_policy.network_partition_key.clone())
        .with_initiator_url(&context.base_url);
    let response = loader
        .request_client()
        .fetch_text_for_worker_blocking_boundary(request)
        .map_err(|error| format!("Failed to load worker script `{resource_url}`: {error}"))?;
    crate::worker::ensure_worker_script_redirect_chain_same_origin(
        &context.base_url,
        &response.redirect_chain,
        &response.final_url,
    )
    .map_err(|message| format!("Failed to construct 'Worker': {message}"))?;
    moli_fetch::ensure_http_status_success(response.final_url.as_str(), response.status, false)
        .map_err(|error| error.to_string())?;
    crate::worker::ensure_worker_script_mime_acceptable(
        &response.final_url,
        &response.headers,
        response.body_bytes(),
    )?;
    let (head, body) = response.into_text_parts();
    let mut final_url = head.final_url;
    final_url.set_fragment(script_url.fragment());
    Ok((final_url.to_string(), body))
}

fn worker_script_inherits_parent_service_worker_controller(script_url: &Url) -> bool {
    script_url.scheme() == "blob"
}

fn reserve_inherited_service_worker_worker_client_for_local_script(
    browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
    script_url: &Url,
    storage_key: String,
    secure_context: bool,
    parent_client_id: ServiceWorkerClientId,
) -> Option<ServiceWorkerClientId> {
    if !worker_script_inherits_parent_service_worker_controller(script_url) {
        return None;
    }
    browser_context_runtime.register_reserved_service_worker_worker_client_inheriting_controller(
        script_url.clone(),
        storage_key,
        ServiceWorkerClientType::DedicatedWorker,
        secure_context,
        parent_client_id,
    )
}

fn reserve_nested_inherited_service_worker_worker_client_for_local_script(
    script_url: &Url,
    context: &NestedWorkerContext,
    secure_context: bool,
) -> Option<ServiceWorkerClientId> {
    let service_worker_runtime = context.service_worker_runtime.as_ref()?;
    let parent_client_id = context.service_worker_client_id?;
    if !worker_script_inherits_parent_service_worker_controller(script_url) {
        return None;
    }
    service_worker_runtime.register_reserved_worker_client_inheriting_controller_from_client(
        script_url.clone(),
        context.creator_storage_key.serialized_storage_key(),
        ServiceWorkerClientType::DedicatedWorker,
        secure_context,
        parent_client_id,
    )
}

pub(in crate::context_bootstrap) fn is_cross_origin_http_worker_script(
    base_url: &Url,
    script_url: &Url,
) -> bool {
    matches!(script_url.scheme(), "http" | "https") && !moli_url::same_origin(base_url, script_url)
}

pub(in crate::context_bootstrap) fn throw_worker_dom_exception(
    scope: &mut v8::PinScope<'_, '_>,
    name: &'static str,
    _code: i32,
    message: &str,
) {
    let exception = crate::context_bootstrap::new_dom_exception_value(scope, message, name);
    scope.throw_exception(exception);
}

pub(in crate::context_bootstrap) fn materialize_worker_script_source(
    script_url: &Url,
) -> Result<Option<String>, String> {
    match script_url.scheme() {
        "data" => {
            let resource_url = worker_script_resource_url(script_url);
            crate::worker::decode_data_url_script_source(
                &resource_url,
                "Failed to construct 'Worker'",
            )
            .map(Some)
        }
        "blob" => {
            crate::blob::object_url_body_and_type(worker_script_resource_url(script_url).as_str())
                .map(|(body, _)| Some(body))
                .ok_or_else(|| {
                    format!(
                        "Failed to construct 'Worker': blob URL `{}` is unavailable.",
                        script_url
                    )
                })
        }
        "http" | "https" => Ok(None),
        _ => Err(format!(
            "Failed to construct 'Worker': URL scheme `{}` is not allowed.",
            script_url.scheme()
        )),
    }
}

fn local_worker_main_script_network_response(
    script_url: &Url,
    source: &str,
) -> crate::protocol_types::NavigationResponse {
    let resource_url = worker_script_resource_url(script_url);
    let mime_type = match script_url.scheme() {
        "blob" => crate::blob::object_url_body_and_type(resource_url.as_str())
            .map(|(_, mime_type)| mime_type),
        "data" => moli_web_mime::data_url_mime_type(resource_url.as_str()),
        _ => None,
    }
    .filter(|mime_type| !mime_type.is_empty())
    .unwrap_or_else(|| "text/javascript".to_owned());
    crate::protocol_types::NavigationResponse::from_text_body(
        resource_url,
        200,
        vec![("content-type".to_owned(), mime_type)],
        source.to_owned(),
    )
}

/// Get the WorkerHandle pointer from a Worker JS object.
pub(super) fn get_worker_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) -> Option<*mut WorkerHandle> {
    let val = get_private_value(scope, worker, WORKER_HANDLE_SLOT)?;
    let external = v8::Local::<v8::External>::try_from(val).ok()?;
    let ptr = external.value() as *mut WorkerHandle;
    if ptr.is_null() { None } else { Some(ptr) }
}

pub(super) fn worker_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) -> Option<DedicatedWorkerId> {
    let value = get_private_value(scope, worker, WORKER_ID_SLOT)?;
    value
        .integer_value(scope)
        .and_then(|id| u64::try_from(id).ok())
        .map(DedicatedWorkerId::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_script_url_query_uses_document_encoding_override() {
        let base = Url::parse("https://example.test/page.html").expect("base url");
        let resolved = resolve_worker_script_url(
            &base,
            "resources/worker.js?\u{00df}",
            Some(encoding_rs::WINDOWS_1252),
        )
        .expect("worker URL should resolve");

        assert_eq!(
            resolved.as_str(),
            "https://example.test/resources/worker.js?%DF"
        );
    }

    #[test]
    fn worker_script_url_query_defaults_to_utf8_without_document_encoding_override() {
        let base = Url::parse("https://example.test/page.html").expect("base url");
        let resolved = resolve_worker_script_url(&base, "resources/worker.js?\u{00df}", None)
            .expect("worker URL should resolve");

        assert_eq!(
            resolved.as_str(),
            "https://example.test/resources/worker.js?%C3%9F"
        );
    }

    #[test]
    fn only_blob_worker_scripts_inherit_parent_service_worker_controller() {
        assert!(worker_script_inherits_parent_service_worker_controller(
            &Url::parse("blob:https://example.test/id").expect("blob url")
        ));
        assert!(!worker_script_inherits_parent_service_worker_controller(
            &Url::parse("data:text/javascript,postMessage(1)").expect("data url")
        ));
        assert!(!worker_script_inherits_parent_service_worker_controller(
            &Url::parse("https://example.test/worker.js").expect("http url")
        ));
    }

    #[test]
    fn worker_script_url_resolves_parseable_non_fetch_schemes() {
        let base = Url::parse("https://example.test/page.html").expect("base url");

        for (input, expected) in [
            ("unsupported:", "unsupported:"),
            ("about:blank", "about:blank"),
            ("javascript:postMessage(1)", "javascript:postMessage(1)"),
        ] {
            let resolved = resolve_worker_script_url(&base, input, None)
                .expect("parseable worker URL should resolve");
            assert_eq!(resolved.as_str(), expected);
            assert!(
                !worker_script_scheme_can_load(&resolved),
                "{resolved} should not be routed to worker script fetch"
            );
        }
    }
}
