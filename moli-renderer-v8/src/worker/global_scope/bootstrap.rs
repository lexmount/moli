//! Global initialization and constructors for dedicated, shared and service workers.

use super::*;

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerGlobalBootstrapPropertiesDeclaration<'scope> {
    #[webapi(slot = WORKER_STATE_SLOT)]
    worker_state: v8::Local<'scope, v8::External>,
    #[webapi(data_property = "self", readonly)]
    self_value: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = "crossOriginIsolated")]
    cross_origin_isolated: bool,
    #[webapi(data_property = "isSecureContext", readonly)]
    is_secure_context: bool,
    #[webapi(data_property = "globalThis")]
    global_this: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerGlobalNameDeclaration {
    #[webapi(data_property, enumerable)]
    name: String,
}

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "DedicatedWorkerGlobalScope", enumerable)]
struct DedicatedWorkerGlobalMethodsDeclaration {
    #[webapi(method, callback = worker_close_callback, length = 0)]
    close: (),
}

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "SharedWorkerGlobalScope", enumerable)]
struct SharedWorkerGlobalMethodsDeclaration {
    #[webapi(method, callback = worker_close_callback, length = 0)]
    close: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct WorkerGlobalCommonOperationsDeclaration {
    #[webapi(
        method = "structuredClone",
        callback = worker_structured_clone_callback,
        length = 1
    )]
    structured_clone: (),
    #[webapi(method, callback = worker_fetch_callback, length = 1)]
    fetch: (),
    #[webapi(method = "importScripts", callback = worker_import_scripts_callback, length = 0)]
    import_scripts: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct WorkerGlobalCreateImageBitmapDeclaration {
    #[webapi(
        method = "createImageBitmap",
        callback = worker_create_image_bitmap_callback,
        length = 1
    )]
    create_image_bitmap: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerGlobalScopeConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "WorkerGlobalScope")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct DedicatedWorkerGlobalScopeConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "DedicatedWorkerGlobalScope")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct SharedWorkerGlobalScopeConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "SharedWorkerGlobalScope")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ServiceWorkerGlobalScopeConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "ServiceWorkerGlobalScope")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerScopePrototypeConstructorDeclaration<'scope> {
    #[webapi(data_property = "constructor")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerPrototypeTagDeclaration {
    #[webapi(to_string_tag, readonly)]
    tag: &'static str,
}

/// Install the `DedicatedWorkerGlobalScope` APIs on the given V8 global object.
///
/// The `state_ptr` is stored as an external in V8 so callbacks can access
/// `WorkerGlobalState`.  Callers must ensure the `Rc<RefCell<WorkerGlobalState>>`
/// outlives the V8 context.
pub(in crate::worker) fn install_worker_global_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: Rc<RefCell<WorkerGlobalState>>,
    worker_templates: Option<&PreparedWorkerGlobalScopeTemplates>,
) -> Result<()> {
    // Store state pointer as an external on the global so callbacks can find it.
    let state_ptr = Rc::into_raw(state.clone()) as *mut c_void;
    let external = v8::External::new(scope, state_ptr);
    // Prevent leak: the Rc was consumed by into_raw, but we still hold `state`.
    // We reconstruct it so the ref-count is correct.  The raw pointer stored in
    // the external is valid as long as the Rc (held by the caller) is alive.
    unsafe { Rc::from_raw(state_ptr as *const RefCell<WorkerGlobalState>) };
    let global_kind = state.borrow().global_kind.clone();

    let (secure_context, cross_origin_isolated) = {
        let state = state.borrow();
        (
            state.secure_context,
            state.secure_context && state.policy_context.cross_origin_isolated,
        )
    };
    WorkerGlobalBootstrapPropertiesDeclaration::new(
        external,
        global,
        cross_origin_isolated,
        secure_context,
        global,
    )
    .initialize(scope, global)
    .map_err(|error| anyhow!("failed to initialize worker global bootstrap properties: {error}"))?;
    install_worker_performance(scope, global)?;
    install_worker_global_scope_constructors(scope, global, &global_kind, worker_templates)?;
    let (_, realm_kind) = worker_global_scope_interface(&global_kind);
    crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
        scope,
        global,
        realm_kind,
        secure_context,
    )?;
    crate::context_bootstrap::install_trusted_types_runtime_state(scope, global)?;
    let require_trusted_types_for_script =
        crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
            &state.borrow().content_security_policies,
        );
    // V8 bypasses the callback for primitive strings unless this fast path is disabled.
    let has_string_code_generation_policy = {
        let state = state.borrow();
        !state.content_security_policies.is_empty()
            || !state.content_security_report_only_policies.is_empty()
    };
    if has_string_code_generation_policy {
        scope
            .get_current_context()
            .set_allow_generation_from_strings(false);
    }
    if require_trusted_types_for_script {
        crate::context_bootstrap::install_trusted_types_eval_runtime_state(scope, global)?;
    }
    crate::context_bootstrap::install_webassembly_runtime_state(scope, global)?;
    if matches!(
        global_kind,
        crate::worker::thread::WorkerGlobalKind::Service { .. }
    ) {
        install_service_worker_extendable_event_constructors(scope, global)?;
    }
    crate::context_bootstrap::initialize_worker_fetch_realm_state(scope, global)?;
    let subtle_crypto_available = secure_context;
    crate::context_bootstrap::initialize_worker_crypto_realm_state(
        scope,
        global,
        subtle_crypto_available,
    )?;
    crate::context_bootstrap::initialize_worker_file_realm_state(scope, global)?;
    install_worker_create_image_bitmap(scope, global)?;
    let identity = state
        .borrow()
        .loader
        .request_client()
        .browser_identity()
        .clone();
    crate::context_bootstrap::install_worker_navigator_runtime_state(
        scope,
        global,
        secure_context,
        &identity,
    )?;
    crate::context_bootstrap::install_worker_indexed_db_runtime_state(scope, global)?;
    crate::context_bootstrap::install_worker_base64_runtime_state(scope, global)?;
    install_simple_event_target_methods(scope, global, WORKER_GLOBAL_LISTENERS_SLOT, false);
    install_simple_event_target_ordered_handlers(scope, global);
    let script_url = state.borrow().current_script_url.clone();
    let origin = script_url
        .as_ref()
        .map(moli_url::origin_ascii_serialization)
        .unwrap_or_else(|| "null".to_owned());
    WorkerGlobalOriginDeclaration::new(origin)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker global origin: {error}"))?;
    let worker_prototype = global_constructor_prototype(scope, "WorkerGlobalScope")
        .ok_or_else(|| anyhow!("WorkerGlobalScope prototype missing"))?;
    WorkerGlobalOriginPrototypeDeclaration::default()
        .initialize(scope, worker_prototype)
        .map_err(|error| anyhow!("failed to initialize WorkerGlobalScope origin: {error}"))?;
    if let Some(script_url) = script_url {
        crate::context_bootstrap::install_worker_location_runtime_state(
            scope,
            global,
            &script_url,
        )?;
        crate::context_bootstrap::install_worker_script_url_runtime_state(
            scope,
            global,
            &script_url,
        )?;
    }

    match &global_kind {
        crate::worker::thread::WorkerGlobalKind::Dedicated { name } => {
            set_worker_global_name_prop(scope, global, name)?;
            DedicatedWorkerGlobalPostMessageDeclaration::default()
                .initialize(scope, global)
                .map_err(|error| anyhow!("failed to initialize worker postMessage: {error}"))?;
        }
        crate::worker::thread::WorkerGlobalKind::Shared { name, .. } => {
            set_worker_global_name_prop(scope, global, name)?;
        }
        crate::worker::thread::WorkerGlobalKind::Service {
            registration_id,
            version_id,
            scope_url,
        } => {
            set_worker_global_name_prop(scope, global, "")?;
            install_service_worker_global_runtime(
                scope,
                global,
                *registration_id,
                *version_id,
                scope_url,
            )?;
        }
    }

    WorkerGlobalCommonOperationsDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker global operations: {error}"))?;

    install_worker_global_event_handler_accessors(scope, global, &global_kind)?;

    // console
    install_console(scope, global)?;

    WorkerGlobalTimerOperationsDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker timers: {error}"))?;

    crate::context_bootstrap::exposed_interfaces::capture_eager_intrinsic_interfaces(
        scope, global, realm_kind,
    )?;
    Ok(())
}

fn set_worker_global_name_prop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<()> {
    // WorkerGlobalScope.name is [Replaceable] readonly in IDL. Browsers expose
    // an own global property whose assignment becomes a normal enumerable data
    // property, which WPT observes through Object.getOwnPropertyDescriptor.
    WorkerGlobalNameDeclaration::new(name.to_owned())
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker global `name`: {error}"))
}

fn install_worker_create_image_bitmap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    WorkerGlobalCreateImageBitmapDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize createImageBitmap: {error}"))
}

fn worker_create_image_bitmap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let reason = worker_dom_exception_value(
        scope,
        "The source image could not be decoded.",
        "InvalidStateError",
    );
    let _ = resolver.reject(scope, reason);
    rv.set(promise.into());
}

pub(in crate::worker) struct PreparedWorkerGlobalScopeTemplates {
    event_target: v8::Global<v8::FunctionTemplate>,
    worker: v8::Global<v8::FunctionTemplate>,
    specific_worker: v8::Global<v8::FunctionTemplate>,
}

impl PreparedWorkerGlobalScopeTemplates {
    pub(in crate::worker) fn global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        v8::Local::new(scope, &self.specific_worker).instance_template(scope)
    }
}

fn worker_global_scope_interface(
    global_kind: &crate::worker::thread::WorkerGlobalKind,
) -> (
    &'static str,
    crate::context_bootstrap::exposed_interfaces::RealmKind,
) {
    use crate::context_bootstrap::exposed_interfaces::RealmKind;
    match global_kind {
        crate::worker::thread::WorkerGlobalKind::Dedicated { .. } => {
            ("DedicatedWorkerGlobalScope", RealmKind::DedicatedWorker)
        }
        crate::worker::thread::WorkerGlobalKind::Shared { .. } => {
            ("SharedWorkerGlobalScope", RealmKind::SharedWorker)
        }
        crate::worker::thread::WorkerGlobalKind::Service { .. } => {
            ("ServiceWorkerGlobalScope", RealmKind::ServiceWorker)
        }
    }
}

pub(in crate::worker) fn prepare_worker_global_scope_templates<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    global_kind: &crate::worker::thread::WorkerGlobalKind,
) -> Result<PreparedWorkerGlobalScopeTemplates> {
    let (interface, realm_kind) = worker_global_scope_interface(global_kind);
    let event_target =
        crate::context_bootstrap::prepare_worker_event_target_template(scope, realm_kind)?;
    // Every object in a [Global] object's prototype chain has an immutable
    // prototype. Install the inherited templates before creating the context.
    event_target.prototype_template(scope).set_immutable_proto();

    let worker = worker_global_scope_template(scope, "WorkerGlobalScope");
    worker.inherit(event_target);
    let specific_worker = worker_global_scope_template(scope, interface);
    specific_worker.inherit(worker);
    specific_worker
        .instance_template(scope)
        .set_immutable_proto();

    Ok(PreparedWorkerGlobalScopeTemplates {
        event_target: v8::Global::new(scope, event_target),
        worker: v8::Global::new(scope, worker),
        specific_worker: v8::Global::new(scope, specific_worker),
    })
}

fn worker_global_scope_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    name: &'static str,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template =
        v8::FunctionTemplate::builder(worker_global_scope_constructor_callback).build(scope);
    template.set_class_name(v8str(scope, name));
    template.prototype_template(scope).set_immutable_proto();
    template
}

fn install_worker_global_scope_constructors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    global_kind: &crate::worker::thread::WorkerGlobalKind,
    worker_templates: Option<&PreparedWorkerGlobalScopeTemplates>,
) -> Result<()> {
    let (interface, _) = worker_global_scope_interface(global_kind);
    web_api_interfaces::initialize(scope, global, interface)?;
    let templates = worker_templates.ok_or_else(|| {
        anyhow!("worker global scope templates were not prepared before context creation")
    })?;
    let event_target_template = v8::Local::new(scope, &templates.event_target);
    let event_target_ctor = event_target_template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to instantiate worker EventTarget constructor"))?;
    let worker_template = v8::Local::new(scope, &templates.worker);
    let worker_ctor = worker_template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to instantiate WorkerGlobalScope constructor"))?;
    worker_ctor
        .set_prototype(scope, event_target_ctor.into())
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to inherit worker EventTarget constructor"))?;
    let worker_proto = constructor_prototype(scope, worker_ctor, "WorkerGlobalScope")?;
    set_worker_to_string_tag(scope, worker_proto, "WorkerGlobalScope");
    WorkerGlobalScopeConstructorGlobalDeclaration::new(worker_ctor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize WorkerGlobalScope global: {error}"))?;

    let specific_template = v8::Local::new(scope, &templates.specific_worker);
    let specific_ctor = specific_template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to instantiate {interface} constructor"))?;
    specific_ctor
        .set_prototype(scope, worker_ctor.into())
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to inherit {interface} constructor"))?;
    let specific_proto = constructor_prototype(scope, specific_ctor, interface)?;
    set_worker_to_string_tag(scope, specific_proto, interface);
    match global_kind {
        crate::worker::thread::WorkerGlobalKind::Dedicated { .. } => {
            DedicatedWorkerGlobalMethodsDeclaration::default().initialize(scope, specific_proto)?;
            DedicatedWorkerGlobalScopeConstructorGlobalDeclaration::new(specific_ctor)
                .initialize(scope, global)?;
        }
        crate::worker::thread::WorkerGlobalKind::Shared { .. } => {
            SharedWorkerGlobalMethodsDeclaration::default().initialize(scope, specific_proto)?;
            SharedWorkerGlobalScopeConstructorGlobalDeclaration::new(specific_ctor)
                .initialize(scope, global)?;
        }
        crate::worker::thread::WorkerGlobalKind::Service { .. } => {
            ServiceWorkerGlobalScopeConstructorGlobalDeclaration::new(specific_ctor)
                .initialize(scope, global)?;
        }
    }
    if !global
        .get_prototype(scope)
        .is_some_and(|prototype| prototype.strict_equals(specific_proto.into()))
    {
        return Err(anyhow!(
            "worker global template did not install {interface}.prototype"
        ));
    }
    Ok(())
}

pub(super) fn ensure_worker_interface_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &'static str,
) -> Result<()> {
    let global = scope.get_current_context().global(scope);
    if global
        .get(scope, v8str(scope, name).into())
        .is_some_and(|value| !value.is_undefined())
    {
        return Ok(());
    }
    let constructor = worker_scope_constructor(scope, name)?;
    let prototype = constructor_prototype(scope, constructor, name)?;
    set_worker_to_string_tag(scope, prototype, name);
    set_prop(scope, global, name, constructor.into())
}

fn worker_scope_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Function>> {
    let constructor = v8::Function::builder(worker_global_scope_constructor_callback)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build {name} constructor"))?;
    constructor.set_name(v8str(scope, name));
    Ok(constructor)
}

pub(super) fn constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Object>> {
    let prototype_key = v8str(scope, "prototype");
    let prototype = constructor
        .get(scope, prototype_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("{name} constructor prototype missing"))?;
    WorkerScopePrototypeConstructorDeclaration::new(constructor)
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize {name}.prototype.constructor: {error}"))?;
    Ok(prototype)
}

pub(super) fn set_worker_to_string_tag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    tag: &'static str,
) {
    let _ = WorkerPrototypeTagDeclaration::new(tag).initialize(scope, prototype);
}

fn worker_global_scope_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(scope, "Illegal constructor.");
}

fn set_prop(
    scope: &mut v8::PinScope<'_, '_>,
    obj: v8::Local<'_, v8::Object>,
    name: &str,
    value: v8::Local<'_, v8::Value>,
) -> Result<()> {
    let key =
        v8_string(scope, name).ok_or_else(|| anyhow!("failed to allocate worker key `{name}`"))?;
    obj.define_own_property(scope, key.into(), value, v8::PropertyAttribute::DONT_ENUM)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to set worker global `{name}`"))
}
