//! EME bindings without a CDM backend. Requests convert their complete WebIDL
//! inputs, enforce the receiver Document's policy, and asynchronously decline
//! access. Interface shells never manufacture keys, sessions or configurations.

use moli_webapi_declare::WebApiFunctionTemplate;

use super::super::{media_queries, new_dom_exception_value};
use crate::{
    native_bridge::JsContextHost,
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value, v8str},
    web_api_interfaces, webidl,
};

mod schema;

pub(super) use schema::MediaKeysRequirement;

const LISTENERS: &str = "__moliMediaKeySessionListeners";
const MESSAGE_HANDLER: &str = "__moliMediaKeySessionOnmessage";
const STATUS_HANDLER: &str = "__moliMediaKeySessionOnkeystatuseschange";
const UNAVAILABLE: &str = "No content decryption module backend is available.";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Navigator, enumerable, receiver)]
struct NavigatorPrototype {
    #[webapi(method, returns_promise, length = 2, callback = request)]
    request_media_key_system_access: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaKeySystemAccess, enumerable, receiver)]
struct AccessPrototype {
    #[webapi(accessor_property, getter = unavailable)]
    key_system: (),
    #[webapi(method, length = 0, callback = unavailable)]
    get_configuration: (),
    #[webapi(method, returns_promise, length = 0, callback = unavailable)]
    create_media_keys: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaKeys, enumerable, receiver)]
struct KeysPrototype {
    #[webapi(method, length = 0, callback = create_session)]
    create_session: (),
    #[webapi(method, returns_promise, length = 0, callback = status_for_policy)]
    get_status_for_policy: (),
    #[webapi(method, returns_promise, length = 1, callback = certificate)]
    set_server_certificate: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaKeySession, enumerable, receiver)]
struct SessionPrototype {
    #[webapi(accessor_property, getter = unavailable)]
    session_id: (),
    #[webapi(accessor_property, getter = unavailable)]
    expiration: (),
    #[webapi(accessor_property, returns_promise, getter = unavailable)]
    closed: (),
    #[webapi(accessor_property, getter = unavailable)]
    key_statuses: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, MESSAGE_HANDLER))]
    onmessage: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, STATUS_HANDLER))]
    onkeystatuseschange: (),
    #[webapi(method, returns_promise, length = 2, callback = generate_request)]
    generate_request: (),
    #[webapi(method, returns_promise, length = 1, callback = load)]
    load: (),
    #[webapi(method, returns_promise, length = 1, callback = update)]
    update: (),
    #[webapi(method, returns_promise, length = 0, callback = unavailable)]
    close: (),
    #[webapi(method, returns_promise, length = 0, callback = unavailable)]
    remove: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLMediaElement, enumerable, receiver)]
struct MediaPrototype {
    #[webapi(accessor_property, getter = media_keys)]
    media_keys: (),
    #[webapi(method, returns_promise, length = 1, callback = set_media_keys)]
    set_media_keys: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "Navigator" => NavigatorPrototype::initialize_prototype_template(scope, prototype),
        "MediaKeySystemAccess" => AccessPrototype::initialize_prototype_template(scope, prototype),
        "MediaKeys" => KeysPrototype::initialize_prototype_template(scope, prototype),
        "MediaKeySession" => SessionPrototype::initialize_prototype_template(scope, prototype),
        "HTMLMediaElement" => MediaPrototype::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn finalize_media_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> anyhow::Result<()> {
    let global = scope.get_current_context().global(scope);
    if !super::super::runtime_state::window_realm_secure_context_available(scope, global) {
        for name in ["mediaKeys", "setMediaKeys"] {
            anyhow::ensure!(
                prototype.delete(scope, v8str(scope, name).into()) == Some(true),
                "failed to remove unavailable HTMLMediaElement.{name} property"
            );
        }
    }
    Ok(())
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver).expect("validated EME receiver")
}

fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::RequestArgs>(scope, &args) else {
        return;
    };
    let navigator = target(scope, args.this());
    let Some(context) = navigator.get_creation_context(scope) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    // SAFETY: the bridge owns this isolate's host. Receiver ownership, rather
    // than the borrowed binding's callee realm, determines document policy.
    let host: &mut JsContextHost = unsafe { &mut *host_ptr };
    if let Some(identity) = host.window_execution_context_identity_for_v8_context(scope, context)
        && host.window_execution_context_identity_is_current(identity)
        && host
            .document_permissions_policy_for_owner(identity.dispatch_scope())
            .is_some_and(|policy| !policy.encrypted_media_enabled())
    {
        webidl::throw_dom_exception(
            scope,
            "SecurityError",
            "Encrypted media is disallowed by permissions policy.",
        );
        return;
    }
    // Method validation follows complete argument/dictionary conversion and
    // policy checking, even for empty keys or an unsupported key system.
    if parsed.key_system.0.is_empty() || parsed.configurations.is_empty() {
        crate::util::throw_type_error(
            scope,
            "A key system and at least one configuration are required.",
        );
        return;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    host.queue_encrypted_media_request_task(
        scope,
        navigator,
        EncryptedMediaRequestTask {
            resolver: v8::Global::new(scope, resolver),
        },
    );
}

pub(crate) struct EncryptedMediaRequestTask {
    resolver: v8::Global<v8::PromiseResolver>,
}

impl EncryptedMediaRequestTask {
    pub(crate) fn invoke(self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let resolver = v8::Local::new(scope, self.resolver);
        let Some(context) = resolver.get_creation_context(scope) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let error = new_dom_exception_value(scope, UNAVAILABLE, "NotSupportedError");
        resolver.reject(scope, error) == Some(true)
    }
}

fn unavailable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    webidl::throw_dom_exception(scope, "NotSupportedError", UNAVAILABLE);
}

fn create_session<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_converted) = webidl::parse_args::<schema::CreateSessionArgs>(scope, &args) else {
        return;
    };
    unavailable(scope, args, rv);
}

fn status_for_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_converted) = webidl::parse_args::<schema::PolicyArgs>(scope, &args) else {
        return;
    };
    unavailable(scope, args, rv);
}

fn certificate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_converted) = webidl::parse_args::<schema::CertificateArgs>(scope, &args) else {
        return;
    };
    unavailable(scope, args, rv);
}

fn generate_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_converted) = webidl::parse_args::<schema::GenerateArgs>(scope, &args) else {
        return;
    };
    unavailable(scope, args, rv);
}

fn load<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_converted) = webidl::parse_args::<schema::LoadArgs>(scope, &args) else {
        return;
    };
    unavailable(scope, args, rv);
}

fn update<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_converted) = webidl::parse_args::<schema::UpdateArgs>(scope, &args) else {
        return;
    };
    unavailable(scope, args, rv);
}

fn media_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::null(scope).into());
}

fn set_media_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<schema::SetKeysArgs>(scope, &args) else {
        return;
    };
    if parsed.keys.is_some() {
        unavailable(scope, args, rv);
    } else {
        // The currently attached keys are null. Reapplying null takes the
        // algorithm's same-object short circuit, with no CDM work or mutation.
        super::super::stream_adapter::set_resolved_promise(
            scope,
            &mut rv,
            v8::undefined(scope).into(),
        );
    }
}

fn handler_metadata(value: &str) -> (&'static str, &'static str) {
    match value {
        MESSAGE_HANDLER => (MESSAGE_HANDLER, "message"),
        STATUS_HANDLER => (STATUS_HANDLER, "keystatuseschange"),
        _ => unreachable!("native EME event handler metadata"),
    }
}

fn handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = target(scope, args.this());
    let (slot, _) = handler_metadata(&args.data().to_rust_string_lossy(scope));
    rv.set(get_private_value(scope, receiver, slot).unwrap_or_else(|| v8::null(scope).into()));
}

fn set_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = target(scope, args.this());
    let (slot, event) = handler_metadata(&args.data().to_rust_string_lossy(scope));
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, receiver, slot, value);
    media_queries::simple_object_event_set_ordered_handler(
        scope, receiver, LISTENERS, event, slot, active,
    );
}
