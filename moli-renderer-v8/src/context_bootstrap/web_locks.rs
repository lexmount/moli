//! Native Web Locks bindings shared by Window and Worker globals.
//!
//! Storage owns coordination; the renderer retains callbacks and resolvers
//! only on their owning event loop. No author timers or JS bootstrap scripts
//! participate in acquisition, cancellation, or release.

use crate::{
    abort_signal_route::{AbortAlgorithm, ResolvedAbortSignal},
    native_bridge::throw_dom_exception,
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value, v8str},
    web_api_interfaces, webidl,
};
use anyhow::Result;
use moli_storage_service::{
    StorageBucketLocator, WebLockClient, WebLockEvent, WebLockMode, WebLockRequestId,
    WebLockRequestKind, WebLockSnapshot,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const MANAGER_ID: &str = "__moliWebLocksManager";
const LOCK_NAME: &str = "__moliWebLockName";
const LOCK_MODE: &str = "__moliWebLockMode";
const COMPLETION_MANAGER: &str = "__moliWebLockCompletionManager";
const COMPLETION_REQUEST: &str = "__moliWebLockCompletionRequest";
const COMPLETION_FULFILLED: &str = "__moliWebLockCompletionFulfilled";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::LockManager)]
struct LockManagerObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = MANAGER_ID)]
    id: v8::Local<'s, v8::BigInt>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Lock)]
struct LockObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = LOCK_NAME)]
    name: v8::Local<'s, v8::String>,
    #[webapi(slot = LOCK_MODE)]
    mode: &'static str,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::LockManager, enumerable, receiver)]
struct LockManagerPrototype {
    #[webapi(method, returns_promise, length = 2, callback = request)]
    request: (),
    #[webapi(method, returns_promise, length = 0, callback = query)]
    query: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Lock, enumerable, receiver)]
struct LockPrototype {
    #[webapi(accessor_property, getter = name)]
    name: (),
    #[webapi(accessor_property, getter = mode)]
    mode: (),
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "LockMode")]
enum LockMode {
    #[default]
    Exclusive,
    Shared,
}

impl LockMode {
    fn native(self) -> WebLockMode {
        match self {
            Self::Exclusive => WebLockMode::Exclusive,
            Self::Shared => WebLockMode::Shared,
        }
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "LockOptions")]
struct LockOptions<'s> {
    #[webidl(default = false)]
    if_available: bool,
    #[webidl(converter = "enum", default = LockMode::Exclusive)]
    mode: LockMode,
    #[webidl(interface = web_api_interfaces::AbortSignal)]
    signal: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = false)]
    steal: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "LockManager.request")]
struct RequestArgs {
    #[webidl(required, converter = "dom_string16")]
    name: Vec<u16>,
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "LockManager.request")]
struct RequestWithOptionsArgs<'s> {
    #[webidl(required, converter = "dom_string16")]
    name: Vec<u16>,
    #[webidl(required, dictionary)]
    options: LockOptions<'s>,
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

pub(crate) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface {
        "LockManager" => LockManagerPrototype::initialize_prototype_template(scope, prototype),
        "Lock" => LockPrototype::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

pub(crate) fn build<'s>(scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Object>> {
    let service = super::storage_buckets::with_storage_bucket_store_entry(scope, |store| {
        store.storage_service()
    });
    let id = if let Some(queue) = crate::worker::web_locks_tasks::queue(scope) {
        let storage = crate::worker::worker_storage_key(scope)
            .map(|key| key.serialized_storage_key())
            .filter(|key| moli_storage_service::storage_bucket_origin_allows_storage(key))
            .and_then(|storage_key| {
                service.map(|service| (service, StorageBucketLocator::Default { storage_key }))
            });
        Some(queue.register(scope, storage))
    } else {
        context_host_ptr_from_global_bridge(scope)
            .and_then(|host| unsafe { &mut *host }.register_web_locks_client(scope, service))
    }
    .unwrap_or(0);
    let prototype = super::ensure_intrinsic_interface_prototype(scope, "LockManager")?;
    let id = v8::BigInt::new_from_u64(scope, id);
    LockManagerObject::new(prototype, id)
        .bind(scope)
        .map_err(Into::into)
}

fn manager_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> Option<Rc<RefCell<WebLocksState>>> {
    let manager = moli_webapi_declare::web_api_object_target(scope, manager)?;
    let id = v8::Local::<v8::BigInt>::try_from(get_private_value(scope, manager, MANAGER_ID)?)
        .ok()?
        .u64_value()
        .0;
    if id == 0 {
        return None;
    }
    if let Some(queue) = crate::worker::web_locks_tasks::queue(scope) {
        queue.state(id)
    } else {
        context_host_ptr_from_global_bridge(scope)
            .and_then(|host| unsafe { &*host }.web_locks_client_state(id))
    }
}

fn available_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
) -> Option<Rc<RefCell<WebLocksState>>> {
    let Some(state) = manager_state(scope, manager) else {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The associated document is not fully active.",
        );
        return None;
    };
    if state.borrow().client.is_none() {
        throw_dom_exception(
            scope,
            "SecurityError",
            18,
            "Web Locks are unavailable for this storage origin.",
        );
        return None;
    }
    Some(state)
}

fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let (name, options, callback) = if args.length() >= 3 {
        let Some(parsed) = webidl::parse_args::<RequestWithOptionsArgs>(scope, &args) else {
            return;
        };
        (parsed.name, parsed.options, parsed.callback)
    } else {
        let Some(parsed) = webidl::parse_args::<RequestArgs>(scope, &args) else {
            return;
        };
        (parsed.name, LockOptions::default(), parsed.callback)
    };
    let manager = args.this();
    let Some(state) = available_state(scope, manager) else {
        return;
    };
    if name.first() == Some(&(b'-' as u16)) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Lock names cannot start with '-'.",
        );
        return;
    }
    if (options.steal && (options.if_available || matches!(options.mode, LockMode::Shared)))
        || (options.signal.is_some() && (options.steal || options.if_available))
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "The lock options cannot be combined.",
        );
        return;
    }
    let signal = options
        .signal
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal));
    if let Some(signal) = signal
        && signal.is_aborted(scope)
    {
        let reason = signal.reason(scope);
        scope.throw_exception(reason);
        return;
    }
    let kind = if options.steal {
        WebLockRequestKind::Steal
    } else if options.if_available {
        WebLockRequestKind::IfAvailable(options.mode.native())
    } else {
        WebLockRequestKind::Wait(options.mode.native())
    };
    let native = state
        .borrow()
        .client
        .as_ref()
        .expect("available client")
        .prepare_request(name.clone(), kind);
    let id = native.id();
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let abort = signal.map(|signal| {
        let data = completion_data(scope, manager, id, false);
        let algorithm = v8::Function::builder(abort_request)
            .data(data.into())
            .build(scope)
            .expect("Web Locks abort algorithm");
        assert!(
            signal.register_weak_algorithm(scope, algorithm),
            "converted AbortSignal must retain its native route"
        );
        PendingAbort {
            signal: v8::Global::new(scope, signal.value()),
            algorithm: AbortAlgorithm::Strong(v8::Global::new(scope, algorithm)),
        }
    });
    state.borrow_mut().pending.insert(
        id.as_u64(),
        Pending {
            id,
            resolver: v8::Global::new(scope, resolver),
            manager: v8::Global::new(scope, manager),
            kind: PendingKind::Request {
                callback: Some(callback),
                name,
                mode: options.mode.native(),
                abort,
                held: false,
                outcome: None,
            },
        },
    );
    native.submit();
    rv.set(resolver.get_promise(scope).into());
}

fn query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(state) = available_state(scope, args.this()) else {
        return;
    };
    let native = state
        .borrow()
        .client
        .as_ref()
        .expect("available client")
        .prepare_query();
    let id = native.id();
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    state.borrow_mut().pending.insert(
        id.as_u64(),
        Pending {
            id,
            resolver: v8::Global::new(scope, resolver),
            manager: v8::Global::new(scope, args.this()),
            kind: PendingKind::Query,
        },
    );
    native.submit();
    rv.set(resolver.get_promise(scope).into());
}

fn name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(target) = moli_webapi_declare::web_api_object_target(scope, args.this())
        && let Some(value) = get_private_value(scope, target, LOCK_NAME)
    {
        rv.set(value);
    }
}

fn mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(target) = moli_webapi_declare::web_api_object_target(scope, args.this())
        && let Some(value) = get_private_value(scope, target, LOCK_MODE)
    {
        rv.set(value);
    }
}

pub(crate) struct WebLocksState {
    client: Option<WebLockClient>,
    pending: HashMap<u64, Pending>,
}

impl WebLocksState {
    pub(crate) fn new(client: Option<WebLockClient>) -> Self {
        Self {
            client,
            pending: HashMap::new(),
        }
    }
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub(crate) fn retire(&mut self) {
        // Abort routes retain weak algorithms, so dropping pending callbacks
        // cannot leave a signal -> algorithm -> retired Context root.
        self.pending.clear();
        self.client.take();
    }
}

struct Pending {
    id: WebLockRequestId,
    resolver: v8::Global<v8::PromiseResolver>,
    manager: v8::Global<v8::Object>,
    kind: PendingKind,
}

enum PendingKind {
    Query,
    Request {
        callback: Option<webidl::WebIdlCallbackFunction>,
        name: Vec<u16>,
        mode: WebLockMode,
        abort: Option<PendingAbort>,
        held: bool,
        outcome: Option<CallbackOutcome>,
    },
}

struct PendingAbort {
    signal: v8::Global<v8::Object>,
    algorithm: AbortAlgorithm,
}

impl PendingAbort {
    fn remove(self, scope: &mut v8::PinScope<'_, '_>) {
        let signal = v8::Local::new(scope, self.signal);
        let algorithm = self
            .algorithm
            .prepare(scope)
            .expect("pending request retains its native abort algorithm");
        if let Some(signal) = ResolvedAbortSignal::resolve(scope, signal) {
            signal.unregister_algorithm(scope, algorithm);
        }
    }
}

struct CallbackOutcome {
    fulfilled: bool,
    value: v8::Global<v8::Value>,
}

pub(crate) fn dispatch(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WebLocksState>>,
    event: WebLockEvent,
) {
    let id = event.request_id();
    match event {
        WebLockEvent::Granted(_) | WebLockEvent::Unavailable(_) => {
            grant(scope, state, id, matches!(event, WebLockEvent::Granted(_)));
        }
        WebLockEvent::Released(_) => {
            let Some(pending) = state.borrow_mut().pending.remove(&id.as_u64()) else {
                return;
            };
            if let PendingKind::Request {
                outcome: Some(outcome),
                ..
            } = pending.kind
            {
                settle(scope, &pending.resolver, outcome);
            }
        }
        WebLockEvent::Stolen(_) => {
            let Some(pending) = state.borrow_mut().pending.remove(&id.as_u64()) else {
                return;
            };
            if let PendingKind::Request {
                abort: Some(abort), ..
            } = pending.kind
            {
                abort.remove(scope);
            }
            let resolver = v8::Local::new(scope, pending.resolver);
            let error = dom_exception(scope, "AbortError", 20, "The lock was stolen.");
            resolver.reject(scope, error);
        }
        WebLockEvent::Snapshot(_, snapshot) => {
            let Some(pending) = state.borrow_mut().pending.remove(&id.as_u64()) else {
                return;
            };
            let resolver = v8::Local::new(scope, pending.resolver);
            let snapshot = snapshot_object(scope, snapshot);
            resolver.resolve(scope, snapshot.into());
        }
    }
}

fn grant(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WebLocksState>>,
    id: WebLockRequestId,
    held: bool,
) {
    let Some(mut pending) = state.borrow_mut().pending.remove(&id.as_u64()) else {
        return;
    };
    let PendingKind::Request {
        callback,
        name,
        mode,
        abort,
        held: is_held,
        ..
    } = &mut pending.kind
    else {
        return;
    };
    if let Some(abort) = abort.take() {
        let signal_value = v8::Local::new(scope, &abort.signal);
        if let Some(signal) = ResolvedAbortSignal::resolve(scope, signal_value)
            && signal.is_aborted(scope)
        {
            let reason = signal.reason(scope);
            let resolver = v8::Local::new(scope, &pending.resolver);
            resolver.reject(scope, reason);
            abort.remove(scope);
            if let Some(client) = &state.borrow().client {
                client.cancel(id);
            }
            return;
        }
        abort.remove(scope);
    }
    *is_held = held;
    let callback = callback
        .take()
        .expect("lock callback is invoked once")
        .prepare(scope);
    let callback_context = callback.relevant_context(scope);
    let argument = {
        let scope = &mut v8::ContextScope::new(scope, callback_context);
        let argument: v8::Local<v8::Value> = if held {
            let prototype =
                super::ensure_intrinsic_interface_prototype(scope, "Lock").expect("Lock prototype");
            let name = v8::String::new_from_two_byte(scope, name, v8::NewStringType::Normal)
                .expect("lock name");
            LockObject::new(prototype, name, mode_token(*mode))
                .bind(scope)
                .expect("native Lock")
                .into()
        } else {
            v8::null(scope).into()
        };
        v8::Global::new(scope, argument)
    };
    let manager = v8::Local::new(scope, &pending.manager);
    state.borrow_mut().pending.insert(id.as_u64(), pending);
    let promise = {
        let scope = &mut v8::ContextScope::new(scope, callback_context);
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        let argument = v8::Local::new(&scope, argument);
        let receiver = v8::undefined(&scope).into();
        let value = crate::callback_invocation::invoke_synchronous_webidl_callback_function(
            &mut scope,
            &callback,
            receiver,
            &[argument],
        );
        if scope.is_execution_terminating() {
            return;
        }
        let resolver = v8::PromiseResolver::new(&scope).expect("callback return Promise");
        if let Some(value) = value {
            let value = v8::Local::new(&scope, value);
            resolver.resolve(&scope, value);
        } else {
            let error = scope
                .exception()
                .unwrap_or_else(|| v8::undefined(&scope).into());
            scope.reset();
            // Reject does not assimilate a thrown thenable.
            resolver.reject(&scope, error);
        }
        v8::Global::new(&scope, resolver.get_promise(&scope))
    };
    let promise = v8::Local::new(scope, promise);
    let fulfilled = completion_function(scope, manager, id, true);
    let rejected = completion_function(scope, manager, id, false);
    promise.then2(scope, fulfilled, rejected);
}

fn completion_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
    id: WebLockRequestId,
    fulfilled: bool,
) -> v8::Local<'s, v8::Object> {
    let data = v8::Object::new(scope);
    set_private_value(scope, data, COMPLETION_MANAGER, manager.into());
    let id = v8::BigInt::new_from_u64(scope, id.as_u64());
    set_private_value(scope, data, COMPLETION_REQUEST, id.into());
    set_private_value(
        scope,
        data,
        COMPLETION_FULFILLED,
        v8::Boolean::new(scope, fulfilled).into(),
    );
    data
}

fn completion_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    manager: v8::Local<'s, v8::Object>,
    id: WebLockRequestId,
    fulfilled: bool,
) -> v8::Local<'s, v8::Function> {
    let data = completion_data(scope, manager, id, fulfilled);
    v8::Function::builder(callback_settled)
        .data(data.into())
        .build(scope)
        .expect("lock completion callback")
}

fn completion_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, u64, bool)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    let manager =
        v8::Local::<v8::Object>::try_from(get_private_value(scope, data, COMPLETION_MANAGER)?)
            .ok()?;
    let id = v8::Local::<v8::BigInt>::try_from(get_private_value(scope, data, COMPLETION_REQUEST)?)
        .ok()?
        .u64_value()
        .0;
    let fulfilled = get_private_value(scope, data, COMPLETION_FULFILLED)?.boolean_value(scope);
    Some((manager, id, fulfilled))
}

fn callback_settled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some((manager, key, fulfilled)) = completion_target(scope, args.data()) else {
        return;
    };
    let Some(state) = manager_state(scope, manager) else {
        return;
    };
    let value = v8::Global::new(scope, args.get(0));
    let release = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending.get_mut(&key) else {
            return;
        };
        let PendingKind::Request { held, outcome, .. } = &mut pending.kind else {
            return;
        };
        *outcome = Some(CallbackOutcome { fulfilled, value });
        (*held).then_some(pending.id)
    };
    if let Some(id) = release {
        if let Some(client) = &state.borrow().client {
            client.release(id);
        }
    } else {
        let pending = state
            .borrow_mut()
            .pending
            .remove(&key)
            .expect("unavailable lock callback");
        if let PendingKind::Request {
            outcome: Some(outcome),
            ..
        } = pending.kind
        {
            settle(scope, &pending.resolver, outcome);
        }
    }
}

fn abort_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some((manager, key, _)) = completion_target(scope, args.data()) else {
        return;
    };
    let Some(state) = manager_state(scope, manager) else {
        return;
    };
    let Some(pending) = state.borrow_mut().pending.remove(&key) else {
        return;
    };
    let PendingKind::Request {
        abort: Some(abort), ..
    } = pending.kind
    else {
        return;
    };
    let signal = v8::Local::new(scope, &abort.signal);
    if let Some(signal) = ResolvedAbortSignal::resolve(scope, signal) {
        let reason = signal.reason(scope);
        let resolver = v8::Local::new(scope, pending.resolver);
        resolver.reject(scope, reason);
    }
    abort.remove(scope);
    if let Some(client) = &state.borrow().client {
        client.cancel(pending.id);
    }
}

fn settle(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: &v8::Global<v8::PromiseResolver>,
    outcome: CallbackOutcome,
) {
    let resolver = v8::Local::new(scope, resolver);
    let value = v8::Local::new(scope, outcome.value);
    if outcome.fulfilled {
        resolver.resolve(scope, value);
    } else {
        resolver.reject(scope, value);
    }
}

fn dom_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &'static str,
    code: i32,
    message: &'static str,
) -> v8::Local<'s, v8::Value> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    throw_dom_exception(&mut scope, name, code, message);
    let error = scope.exception().expect("native DOMException");
    scope.reset();
    error
}

fn mode_token(mode: WebLockMode) -> &'static str {
    match mode {
        WebLockMode::Exclusive => "exclusive",
        WebLockMode::Shared => "shared",
    }
}

fn snapshot_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: WebLockSnapshot,
) -> v8::Local<'s, v8::Object> {
    let result = v8::Object::new(scope);
    for (key, entries) in [("held", snapshot.held), ("pending", snapshot.pending)] {
        let entries: Vec<_> = entries
            .into_iter()
            .map(|info| {
                let object = v8::Object::new(scope);
                let name =
                    v8::String::new_from_two_byte(scope, &info.name, v8::NewStringType::Normal)
                        .expect("lock name");
                object.create_data_property(scope, v8str(scope, "name").into(), name.into());
                object.create_data_property(
                    scope,
                    v8str(scope, "mode").into(),
                    v8str(scope, mode_token(info.mode)).into(),
                );
                object.create_data_property(
                    scope,
                    v8str(scope, "clientId").into(),
                    v8::String::new(scope, &info.client_id)
                        .expect("client ID")
                        .into(),
                );
                object.into()
            })
            .collect();
        let array = v8::Array::new_with_elements(scope, &entries);
        result.create_data_property(scope, v8str(scope, key).into(), array.into());
    }
    result
}
