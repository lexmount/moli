use super::{
    CHILD_BROWSING_CONTEXT_HANDLE_SLOT, MessagePortRealmBinding,
    ensure_message_port_wrapper_for_id_in_realm,
    events::{clear_event_dispatch_fields, set_event_dispatch_fields},
    invoke_simple_event_listener,
    navigation_serialize::{
        current_document_content_security_policies, current_document_referrer_policy,
        document_referrer_policy_for_native_document,
    },
    shared::{SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, SIMPLE_EVENT_TARGET_SLOT},
    simple_event_target_add_event_listener_callback, simple_event_target_dispatch_event_callback,
    simple_event_target_remove_event_listener_callback, simple_object_event_listeners_snapshot,
    simple_object_event_remove_listener_value_for_type, simple_object_event_set_ordered_handler,
    worker_host::{
        document_query_encoding_override, is_cross_origin_http_worker_script,
        materialize_worker_script_source, resolve_worker_script_url, throw_worker_dom_exception,
        trusted_worker_script_url_string_or_throw, worker_constructor_base_url,
        worker_script_resource_url, worker_script_scheme_can_load,
    },
};
use crate::{
    context_bootstrap::{SharedStorageBucketStore, WeakIndexedDbManager},
    document_runtime::DomHandle,
    native_bridge::WorkerOwnerScope,
    network::ResourceRequestClient,
    page_task_queue::RendererWorkerHostBridgeEventSender,
    runtime::RendererBrowserContextRuntime,
    shared_worker_runtime::{
        SharedWorkerClientEndpointReceiver, SharedWorkerClientError,
        SharedWorkerClientFrameIdentity, SharedWorkerExecutionPolicy, SharedWorkerLaunchContext,
        SharedWorkerLaunchParams, SharedWorkerScriptLoad, SharedWorkerScriptRequestPolicy,
    },
    types::SubresourcePolicyContext,
    util::{
        context_host_ptr_from_global_bridge, get_private_value, set_private_value,
        throw_type_error, v8_string, v8str,
    },
    worker::{WorkerNetworkPolicy, WorkerScriptKind, worker_secure_context_for_script_url},
};
use moli_shared_worker::{
    SharedWorkerCreationContextType, SharedWorkerCredentialsMode, SharedWorkerDescriptor,
    SharedWorkerKey, SharedWorkerSameSiteCookies, SharedWorkerScriptType,
};
use moli_storage_key::MoliStorageKey;
use moli_webapi_declare::WebApiObject;
use url::Url;

const SHARED_WORKER_LISTENERS_SLOT: &str = "__moliSharedWorkerListeners";
const SHARED_WORKER_CLIENT_ID_SLOT: &str = "__moliSharedWorkerClientId";
const SHARED_WORKER_ONERROR_SLOT: &str = "__moliSharedWorkerOnError";

#[derive(WebApiObject)]
#[webapi(interface = "SharedWorker")]
struct SharedWorkerObjectDeclaration<'scope> {
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = SHARED_WORKER_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(slot = SHARED_WORKER_ONERROR_SLOT, init = "null")]
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
        getter = shared_worker_onerror_getter,
        setter = shared_worker_onerror_setter
    )]
    onerror: (),

    #[webapi(data_property, readonly)]
    port: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct SharedWorkerHostEventInitDeclaration {
    #[webapi(data_property, enumerable)]
    cancelable: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct SharedWorkerHostEventFallbackDeclaration {
    #[webapi(data_property, enumerable)]
    r#type: String,
    #[webapi(data_property, enumerable)]
    cancelable: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct SharedWorkerHostErrorEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    message: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    filename: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct SharedWorkerHostErrorEventFallbackDeclaration<'scope, 'text> {
    #[webapi(data_property, enumerable)]
    r#type: &'static str,
    #[webapi(data_property, enumerable)]
    message: &'text str,
    #[webapi(data_property, enumerable)]
    filename: &'text str,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope, data_properties, enumerable)]
struct SharedWorkerHostErrorEventDetailsDeclaration<'scope, 'text> {
    message: &'text str,
    filename: &'text str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'scope, v8::Value>,
}

struct SharedWorkerOptions {
    name: String,
    script_kind: WorkerScriptKind,
    credentials_mode: SharedWorkerCredentialsMode,
    same_site_cookies: Option<SharedWorkerSameSiteCookies>,
}

struct SharedWorkerConstructorContext {
    base_url: Url,
    query_encoding: Option<&'static encoding_rs::Encoding>,
    request_client: ResourceRequestClient,
    resource_task_runner: crate::network::RendererResourceTaskRunner,
    network_policy: WorkerNetworkPolicy,
    policy_context: SubresourcePolicyContext,
    storage_key: MoliStorageKey,
    top_level_site: String,
    document_referrer_policy: Option<String>,
    document_content_security_policies: Vec<String>,
    browser_context_runtime: RendererBrowserContextRuntime,
    indexed_db_manager: Option<WeakIndexedDbManager>,
    storage_bucket_store: SharedStorageBucketStore,
    client_identity: SharedWorkerClientFrameIdentity,
    worker_owner_child_handle: Option<DomHandle>,
    client_event_realm: crate::page_task_queue::RendererPageSharedWorkerClientEventRealmSender,
    worker_host_bridge_sender: RendererWorkerHostBridgeEventSender,
}

impl Default for SharedWorkerOptions {
    fn default() -> Self {
        Self {
            name: String::new(),
            script_kind: WorkerScriptKind::Classic,
            credentials_mode: SharedWorkerCredentialsMode::SameOrigin,
            same_site_cookies: None,
        }
    }
}

fn shared_worker_onerror_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), SHARED_WORKER_ONERROR_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

fn shared_worker_onerror_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, args.this(), SHARED_WORKER_ONERROR_SLOT, stored);
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        SHARED_WORKER_LISTENERS_SLOT,
        "error",
        SHARED_WORKER_ONERROR_SLOT,
        stored.is_function(),
    );
}

pub(in crate::context_bootstrap) fn shared_worker_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    shared_worker_constructor_callback_inner(scope, args, rv);
}

fn shared_worker_constructor_callback_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'SharedWorker': Please use the 'new' operator.",
        );
        return;
    }
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to construct 'SharedWorker': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(options) = parse_shared_worker_options(scope, &args) else {
        return;
    };
    let Some((context, host_ptr)) = shared_worker_constructor_context(scope, args.this()) else {
        throw_type_error(
            scope,
            "Failed to construct 'SharedWorker': constructor context is unavailable.",
        );
        return;
    };
    let require_trusted_types_for_script =
        crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
            &context.document_content_security_policies,
        );
    let Some(script_url_input) = trusted_worker_script_url_string_or_throw(
        scope,
        args.get(0),
        crate::content_security_policy::TrustedTypesForScriptRequirements::enforced_only(
            require_trusted_types_for_script,
        ),
        "SharedWorker constructor",
        "SharedWorker",
    ) else {
        return;
    };
    let resolved_url = match resolve_worker_script_url(
        &context.base_url,
        &script_url_input,
        context.query_encoding,
    ) {
        Ok(url) => url,
        Err(message) => {
            throw_worker_dom_exception(scope, "SyntaxError", 12, &message);
            return;
        }
    };
    let creation_context_type = if context.network_policy.secure_context {
        SharedWorkerCreationContextType::Secure
    } else {
        SharedWorkerCreationContextType::Nonsecure
    };
    let script_type = match options.script_kind {
        WorkerScriptKind::Classic => SharedWorkerScriptType::Classic,
        WorkerScriptKind::Module => SharedWorkerScriptType::Module,
    };
    let descriptor =
        SharedWorkerDescriptor::new(script_type, options.credentials_mode, creation_context_type);
    let same_site_cookies = options.same_site_cookies.unwrap_or_else(|| {
        SharedWorkerSameSiteCookies::default_for_storage_key(&context.storage_key)
    });
    if !same_site_cookies.is_allowed_for_storage_key(&context.storage_key) {
        throw_worker_dom_exception(
            scope,
            "SecurityError",
            18,
            "SharedWorkers in third-party contexts cannot request SameSite Strict or Lax cookies via the `sameSiteCookies: \"all\"` option.",
        );
        return;
    }
    let script_request_policy = SharedWorkerScriptRequestPolicy::from_descriptor(
        &descriptor,
        same_site_cookies,
        context.document_referrer_policy.clone(),
        context.network_policy.network_partition_key.clone(),
        context.document_content_security_policies.clone(),
    );
    let module_credentials_mode = script_request_policy.credentials_mode();
    let script_load = match prepare_shared_worker_script_load(
        context.request_client.clone(),
        context.resource_task_runner.clone(),
        &context.base_url,
        &resolved_url,
        script_request_policy,
    ) {
        Ok(source) => source,
        Err(message) => {
            throw_type_error(scope, &message);
            return;
        }
    };
    let message_port_registry = context.browser_context_runtime.message_port_registry();
    let Some(message_port_realm) = MessagePortRealmBinding::current(scope) else {
        throw_type_error(
            scope,
            "Failed to construct 'SharedWorker': Window execution context is unavailable.",
        );
        return;
    };
    let (client_port_id, worker_port_id) =
        message_port_registry.create_entangled_message_port_pair(message_port_realm.owner());
    message_port_registry.detach_message_port_owner_for_transfer(worker_port_id);
    let Some(client_port) =
        ensure_message_port_wrapper_for_id_in_realm(scope, client_port_id, &message_port_realm)
    else {
        message_port_registry.close_message_port(client_port_id);
        message_port_registry.close_message_port(worker_port_id);
        throw_type_error(
            scope,
            "Failed to construct 'SharedWorker': MessagePort wrapper is unavailable.",
        );
        return;
    };
    let worker = args.this();
    SharedWorkerObjectDeclaration::new(client_port)
        .initialize(scope, worker)
        .expect("SharedWorker declaration should initialize");

    let script_url = resolved_url.to_string();
    let key = SharedWorkerKey::new(
        context.storage_key,
        script_url.clone(),
        options.name.clone(),
        same_site_cookies,
    );
    let parent_service_worker_client_id = shared_worker_parent_service_worker_client_id_for_script(
        scope,
        unsafe { &mut *host_ptr },
        context.worker_owner_child_handle,
        &resolved_url,
    );
    let mut network_policy = context.network_policy;
    network_policy.secure_context =
        worker_secure_context_for_script_url(&resolved_url, network_policy.secure_context);
    let execution_policy = SharedWorkerExecutionPolicy::new(
        options.script_kind,
        context.base_url.clone(),
        context.document_content_security_policies.clone(),
        network_policy,
        context.policy_context,
        context.browser_context_runtime.worker_context_runtime(),
        Some(context.top_level_site),
        module_credentials_mode,
    )
    .with_service_worker_runtime(context.browser_context_runtime.service_worker_runtime())
    .with_indexed_db_manager(context.indexed_db_manager)
    .with_storage_bucket_store(context.storage_bucket_store);
    let launch_context =
        SharedWorkerLaunchContext::new(options.name, context.request_client, execution_policy);
    let params = SharedWorkerLaunchParams::new(
        key,
        script_load,
        launch_context,
        client_port_id,
        worker_port_id,
        context.client_identity.owner_id(),
        parent_service_worker_client_id,
        context.client_event_realm,
        context.worker_host_bridge_sender,
    );
    let client_id = context
        .browser_context_runtime
        .connect_shared_worker(descriptor, params);
    set_private_value(
        scope,
        worker,
        SHARED_WORKER_CLIENT_ID_SLOT,
        v8::Number::new(scope, client_id.as_u64() as f64).into(),
    );
    unsafe { &mut *host_ptr }.register_shared_worker_client(
        scope,
        SharedWorkerClientEndpointReceiver::new(
            client_id,
            context.client_identity,
            context.browser_context_runtime.clone(),
        ),
        worker,
    );

    rv.set(worker.into());
}

fn shared_worker_constructor_context(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
) -> Option<(
    SharedWorkerConstructorContext,
    *mut crate::native_bridge::JsContextHost,
)> {
    let (current_context_document_referrer_policy, document_meta_content_security_policies) = {
        let global = scope.get_current_context().global(scope);
        (
            current_document_referrer_policy(scope, global),
            current_document_content_security_policies(scope, global),
        )
    };
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        let child_handle = current_child_context_handle(scope)
            .or_else(|| child_context_handle_from_object_creation_context(scope, receiver))
            .or_else(|| {
                host.active_child_subresource_request_scope()
                    .map(|(handle, _, _)| handle)
            });
        let active_popup_referrer_policy = child_handle
            .is_none()
            .then(|| {
                host.active_lightweight_popup_referrer_policy(scope)
                    .map(ToOwned::to_owned)
            })
            .flatten();
        let document_referrer_policy = child_handle
            .and_then(|handle| host.child_browsing_context_document_handle(handle))
            .and_then(|document_handle| {
                document_referrer_policy_for_native_document(host, document_handle)
            })
            .or(active_popup_referrer_policy)
            .or(current_context_document_referrer_policy);
        let storage_context = host.active_storage_context(scope, child_handle);
        let network_partition_key = child_handle
            .and_then(|handle| host.child_browsing_context_network_partition_key(handle));
        let owner_scope = child_handle
            .map(crate::native_bridge::OwnerDispatchScope::Child)
            .or_else(|| {
                crate::native_bridge::active_lightweight_popup_id(scope)
                    .map(crate::native_bridge::OwnerDispatchScope::LightweightPopup)
            })
            .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
        let policy_context =
            crate::network_host::effective_subresource_policy_context(scope, host, owner_scope);
        let active_popup_base_url = child_handle
            .is_none()
            .then(|| host.active_lightweight_popup_base_url(scope))
            .flatten();
        let base_url = child_handle
            .and_then(|handle| host.child_browsing_context_base_url(handle))
            .or(active_popup_base_url)
            .unwrap_or_else(|| worker_constructor_base_url(host));
        let storage_key = storage_context.storage_key().clone();
        let top_level_site = storage_key.top_level_site().to_owned();
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
        let client_owner_id = child_handle
            .map(|handle| host.shared_worker_client_owner_id_for_child_context(handle))
            .unwrap_or_else(|| host.shared_worker_client_owner_id());
        let execution_context = host.current_runtime_window_execution_context_identity(scope)?;
        let client_identity =
            SharedWorkerClientFrameIdentity::new(client_owner_id, execution_context);
        let client_event_realm = host
            .page_shared_worker_client_event_sender()
            .bind_execution_context(execution_context);
        let worker_host_bridge_sender = host.page_worker_host_bridge_event_sender().clone();
        let secure_context_url = child_handle
            .and_then(|handle| host.child_browsing_context_secure_context_url(handle))
            .or_else(|| host.active_lightweight_popup_base_url(scope))
            .unwrap_or_else(|| host.document_url().clone());
        let creator_secure_context = moli_url::is_potentially_trustworthy_url(&secure_context_url);
        let resource_loader = host.document_resource_loader_for_dispatch_scope(owner_scope)?;
        let context = SharedWorkerConstructorContext {
            base_url,
            query_encoding: document_query_encoding_override(host),
            request_client: resource_loader.request_client().clone(),
            resource_task_runner: resource_loader.task_runner(),
            network_policy: WorkerNetworkPolicy {
                secure_context: creator_secure_context,
                permission_overrides: host.permission_overrides().to_vec(),
                extra_http_headers: host.extra_http_headers().to_vec(),
                network_offline: host.network_offline(),
                blocked_url_patterns: host.blocked_url_patterns().to_vec(),
                network_partition_key,
                fetch_subresource_interception_enabled: host
                    .fetch_subresource_interception_enabled(),
                fetch_subresource_interception_resource_type: host
                    .fetch_subresource_interception_resource_type(),
            },
            policy_context,
            storage_key,
            top_level_site,
            document_referrer_policy,
            document_content_security_policies,
            browser_context_runtime: host.browser_context_runtime(),
            indexed_db_manager: host.indexed_db_manager(),
            storage_bucket_store: host.storage_bucket_store(),
            client_identity,
            worker_owner_child_handle: child_handle,
            client_event_realm,
            worker_host_bridge_sender,
        };
        return Some((context, host_ptr));
    }

    None
}

fn shared_worker_parent_service_worker_client_id_for_script(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut crate::native_bridge::JsContextHost,
    child_handle: Option<DomHandle>,
    script_url: &Url,
) -> Option<crate::service_worker_runtime::ServiceWorkerClientId> {
    if script_url.scheme() != "blob" {
        return None;
    }
    let worker_owner_scope = child_handle
        .map(WorkerOwnerScope::Child)
        .or_else(|| {
            crate::native_bridge::active_lightweight_popup_id(scope)
                .map(WorkerOwnerScope::LightweightPopup)
        })
        .unwrap_or(WorkerOwnerScope::Top);
    Some(host.service_worker_client_id_for_worker_owner(worker_owner_scope))
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
    if let Some(handle) = get_private_value(scope, global, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| {
            if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
                let (index, lossless) = big.u64_value();
                return lossless.then(|| DomHandle::new(index as usize));
            }
            value.number_value(scope).and_then(|n| {
                (n.is_finite() && n >= 0.0 && n.fract() == 0.0).then(|| DomHandle::new(n as usize))
            })
        })
    {
        return Some(handle);
    }
    let key = v8str(scope, CHILD_BROWSING_CONTEXT_HANDLE_SLOT);
    let value = global.get(scope, key.into())?;
    let n = value.number_value(scope)?;
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 {
        Some(DomHandle::new(n as usize))
    } else {
        None
    }
}

fn parse_shared_worker_options(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<SharedWorkerOptions> {
    if args.length() < 2 {
        return Some(SharedWorkerOptions::default());
    }
    let value = args.get(1);
    if value.is_undefined() || value.is_null() {
        return Some(SharedWorkerOptions::default());
    }
    if value.is_string() {
        return Some(SharedWorkerOptions {
            name: value.to_string(scope)?.to_rust_string_lossy(scope),
            ..SharedWorkerOptions::default()
        });
    }
    if !value.is_object() {
        return Some(SharedWorkerOptions {
            name: value.to_string(scope)?.to_rust_string_lossy(scope),
            ..SharedWorkerOptions::default()
        });
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "Failed to construct 'SharedWorker': options must be a string or object.",
        );
        return None;
    };
    let name = optional_string_property(scope, object, "name")?.unwrap_or_default();
    let script_kind = match optional_string_property(scope, object, "type")? {
        Some(value) if value == "classic" => WorkerScriptKind::Classic,
        Some(value) if value == "module" => WorkerScriptKind::Module,
        Some(_) => {
            throw_type_error(
                scope,
                "Failed to construct 'SharedWorker': the provided worker type is invalid.",
            );
            return None;
        }
        None => WorkerScriptKind::Classic,
    };
    let credentials_mode = match optional_string_property(scope, object, "credentials")? {
        Some(value) if value == "omit" => SharedWorkerCredentialsMode::Omit,
        Some(value) if value == "same-origin" => SharedWorkerCredentialsMode::SameOrigin,
        Some(value) if value == "include" => SharedWorkerCredentialsMode::Include,
        Some(_) => {
            throw_type_error(
                scope,
                "Failed to construct 'SharedWorker': the provided credentials mode is invalid.",
            );
            return None;
        }
        None => SharedWorkerCredentialsMode::SameOrigin,
    };
    let same_site_cookies = match optional_string_property(scope, object, "sameSiteCookies")? {
        Some(value) if value == "all" => Some(SharedWorkerSameSiteCookies::All),
        Some(value) if value == "none" => Some(SharedWorkerSameSiteCookies::None),
        Some(_) => {
            throw_type_error(
                scope,
                "Failed to construct 'SharedWorker': the provided sameSiteCookies mode is invalid.",
            );
            return None;
        }
        None => None,
    };
    Some(SharedWorkerOptions {
        name,
        script_kind,
        credentials_mode,
        same_site_cookies,
    })
}

fn optional_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<Option<String>> {
    let key = v8_string(scope, key)?;
    let value = object.get(scope, key.into())?;
    if value.is_undefined() {
        return Some(None);
    }
    Some(Some(value.to_string(scope)?.to_rust_string_lossy(scope)))
}

fn prepare_shared_worker_script_load(
    request_client: ResourceRequestClient,
    resource_task_runner: crate::network::RendererResourceTaskRunner,
    base_url: &Url,
    script_url: &Url,
    request_policy: SharedWorkerScriptRequestPolicy,
) -> Result<SharedWorkerScriptLoad, String> {
    if let Err(message) = request_policy.ensure_allows_script_url(base_url, script_url) {
        return Ok(SharedWorkerScriptLoad::failure(message));
    }
    if script_url.scheme() == "data" {
        let resource_url = worker_script_resource_url(script_url);
        return Ok(
            match crate::worker::decode_data_url_script_source(
                &resource_url,
                "Failed to load shared worker script",
            ) {
                Ok(source) => SharedWorkerScriptLoad::ready(script_url.to_string(), source),
                Err(message) => SharedWorkerScriptLoad::failure(message),
            },
        );
    }
    if script_url.scheme() == "blob" {
        return Ok(SharedWorkerScriptLoad::blob(script_url.clone()));
    }
    if !worker_script_scheme_can_load(script_url) {
        return Ok(SharedWorkerScriptLoad::failure(format!(
            "Failed to load shared worker script `{script_url}`: URL scheme `{}` is not allowed.",
            script_url.scheme()
        )));
    }
    if let Some(source) = materialize_worker_script_source(script_url)? {
        return Ok(SharedWorkerScriptLoad::ready(
            script_url.to_string(),
            source,
        ));
    }
    if is_cross_origin_http_worker_script(base_url, script_url) {
        return Ok(SharedWorkerScriptLoad::failure(format!(
            "Failed to load shared worker script `{script_url}`: cross-origin worker script blocked."
        )));
    }
    Ok(SharedWorkerScriptLoad::fetch(
        request_client,
        resource_task_runner,
        script_url.clone(),
        base_url.clone(),
        request_policy,
    ))
}

pub(crate) fn dispatch_shared_worker_client_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    error_event: &SharedWorkerClientError,
) -> bool {
    let error = v8::null(scope).into();
    dispatch_shared_worker_error_event(
        scope,
        worker,
        error_event.message(),
        error_event.filename(),
        error_event.lineno(),
        error_event.colno(),
        error,
        error_event.event_kind(),
    )
}

fn dispatch_shared_worker_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
    event_kind: crate::worker::WorkerParentErrorEventKind,
) -> bool {
    let event = match event_kind {
        crate::worker::WorkerParentErrorEventKind::Event => {
            let event = new_event(scope, "error", true);
            set_error_event_details(scope, event, message, filename, lineno, colno, error);
            event
        }
        crate::worker::WorkerParentErrorEventKind::ErrorEvent => {
            new_error_event(scope, message, filename, lineno, colno, error)
        }
    };
    set_event_dispatch_fields(scope, worker, event);

    let listeners = simple_object_event_listeners_snapshot(
        scope,
        worker,
        SHARED_WORKER_LISTENERS_SLOT,
        "error",
    );
    let mut once_listeners = Vec::new();
    let mut dispatched = false;
    for listener in listeners {
        dispatched = true;
        let callback_result = invoke_simple_event_listener(
            scope,
            "error",
            "SharedWorker error listener",
            &listener,
            worker.into(),
            &[event.into()],
            event,
        );
        if listener.handler_slot.as_deref() == Some(SHARED_WORKER_ONERROR_SLOT)
            && let Some(returned) = callback_result
            && v8::Local::new(scope, &returned).boolean_value(scope)
        {
            let _ = event.set(
                scope,
                v8str(scope, "defaultPrevented").into(),
                v8::Boolean::new(scope, true).into(),
            );
        }
        if listener.once {
            once_listeners.push(listener.original);
        }
    }

    for listener in once_listeners {
        simple_object_event_remove_listener_value_for_type(
            scope,
            worker,
            SHARED_WORKER_LISTENERS_SLOT,
            "error",
            listener,
            false,
        );
    }

    clear_event_dispatch_fields(scope, event);
    dispatched
}

fn new_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    cancelable: bool,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(event_ctor) = global
        .get(scope, v8str(scope, "Event").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let init = SharedWorkerHostEventInitDeclaration::new(cancelable)
            .bind(scope)
            .expect("SharedWorker host Event init declaration should bind");
        if let Some(event) = event_ctor.new_instance(
            scope,
            &[
                v8::String::new(scope, event_type).unwrap().into(),
                init.into(),
            ],
        ) {
            return event;
        }
    }

    SharedWorkerHostEventFallbackDeclaration::new(event_type.to_owned(), cancelable)
        .bind(scope)
        .expect("SharedWorker host Event fallback declaration should bind")
}

fn new_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(error_ctor) = global
        .get(scope, v8str(scope, "ErrorEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let init = SharedWorkerHostErrorEventInitDeclaration::new(
            v8::String::new(scope, message).unwrap(),
            v8::String::new(scope, filename).unwrap(),
            lineno,
            colno,
            true,
            error,
        )
        .bind(scope)
        .expect("SharedWorker host ErrorEvent init declaration should bind");
        if let Some(event) = error_ctor.new_instance(
            scope,
            &[v8::String::new(scope, "error").unwrap().into(), init.into()],
        ) {
            return event;
        }
    }

    SharedWorkerHostErrorEventFallbackDeclaration::new(
        "error", message, filename, lineno, colno, error,
    )
    .bind(scope)
    .expect("SharedWorker host ErrorEvent fallback declaration should bind")
}

fn set_error_event_details<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
) {
    SharedWorkerHostErrorEventDetailsDeclaration::new(message, filename, lineno, colno, error)
        .initialize(scope, event)
        .expect("SharedWorker host ErrorEvent details declaration should initialize");
}
