//! Secure Window wake-lock bindings for a UA without a platform wake-lock backend.
//!
//! The specification permits implementation-specific denial. Convert arguments
//! before rejecting valid requests; never manufacture an acquired sentinel.

use anyhow::Result;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::{native_bridge::throw_dom_exception, web_api_interfaces, webidl};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WakeLock)]
struct WakeLockObjectDeclaration<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WakeLock, enumerable, receiver)]
struct WakeLockPrototypeDeclaration {
    #[webapi(method, returns_promise, length = 0, callback = request)]
    request: (),
}

// request() cannot return a sentinel until platform acquisition is implemented.
// Keep the native interface surface and receiver validation without inventing
// acquired/released state or dispatching a synthetic release event.
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WakeLockSentinel, enumerable, receiver)]
struct WakeLockSentinelPrototypeDeclaration {
    #[webapi(accessor_property, getter = sentinel_backend_unavailable)]
    released: (),
    #[webapi(accessor_property = "type", getter = sentinel_backend_unavailable)]
    lock_type: (),
    #[webapi(method, returns_promise, length = 0, callback = sentinel_backend_unavailable)]
    release: (),
    #[webapi(accessor_property, getter = sentinel_backend_unavailable, setter = sentinel_backend_unavailable)]
    onrelease: (),
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "WakeLockType")]
enum WakeLockType {
    Screen,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WakeLock.request")]
struct RequestArgs {
    #[webidl(converter = "enum", default = WakeLockType::Screen)]
    lock_type: WakeLockType,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "WakeLock" => {
            WakeLockPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "WakeLockSentinel" => {
            WakeLockSentinelPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(super) fn build<'s>(scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Object>> {
    let prototype =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, "WakeLock")?;
    WakeLockObjectDeclaration::new(prototype)
        .bind(scope)
        .map_err(Into::into)
}

fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<RequestArgs>(scope, &args) else {
        return;
    };
    match parsed.lock_type {
        WakeLockType::Screen => throw_dom_exception(
            scope,
            "NotAllowedError",
            0,
            "Screen wake locks are unavailable on this platform.",
        ),
    }
}

fn sentinel_backend_unavailable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "Platform wake-lock sentinels are not implemented.",
    );
}
