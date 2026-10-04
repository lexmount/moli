//! Native credential-manager entry points without a credential-store backend.
//!
//! WebIDL conversion and fully-active/pre-aborted checks still apply when no
//! credential can be produced. The generated Promise adapter handles receiver
//! and conversion errors in the function realm. Do not manufacture credentials
//! or claim to persist them.

mod options;

use anyhow::Result;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::{
    native_bridge::{OwnerDispatchScope, document::document_is_fully_active, throw_dom_exception},
    util::context_host_ptr_from_context_slot,
    web_api_interfaces, webidl,
};

use options::{CreateArgs, GetArgs, StoreArgs};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CredentialsContainer)]
struct ContainerObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CredentialsContainer, enumerable, receiver)]
struct ContainerPrototype {
    #[webapi(method, returns_promise, length = 0, callback = get)]
    get: (),
    #[webapi(method, returns_promise, length = 1, callback = store)]
    store: (),
    #[webapi(method, returns_promise, length = 0, callback = create)]
    create: (),
    #[webapi(method, returns_promise, length = 0, callback = prevent_silent_access)]
    prevent_silent_access: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    ContainerPrototype::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap::navigator_runtime) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    let prototype = crate::context_bootstrap::ensure_intrinsic_interface_prototype(
        scope,
        "CredentialsContainer",
    )?;
    ContainerObject::new(prototype)
        .bind(scope)
        .map_err(Into::into)
}

fn check_fully_active(scope: &mut v8::PinScope<'_, '_>) -> bool {
    // Current settings come from the function realm, never from an author
    // property such as window.document or a retained WindowProxy association.
    let context = scope.get_current_context();
    let fully_active = context_host_ptr_from_context_slot(context).is_some_and(|host_ptr| {
        let host = unsafe { &*host_ptr };
        let Some(identity) = host.window_execution_context_identity_for_access_check(context)
        else {
            return false;
        };
        if !host.window_execution_context_identity_is_current(identity) {
            return false;
        }
        let document = match identity.dispatch_scope() {
            OwnerDispatchScope::Top => Some(host.document_handle()),
            OwnerDispatchScope::Child(frame) => host.child_browsing_context_document_handle(frame),
            OwnerDispatchScope::LightweightPopup(id) => host.lightweight_popup_document_handle(id),
        };
        document.is_some_and(|document| document_is_fully_active(host, document))
    });
    if !fully_active {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The document is not fully active.",
        );
    }
    fully_active
}

fn check_aborted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: Option<options::Signal<'s>>,
) -> bool {
    if let Some(signal) = signal
        && signal.0.is_aborted(scope)
    {
        let reason = signal.0.reason(scope);
        scope.throw_exception(reason);
        return true;
    }
    false
}

fn backend_unavailable(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "No supported credential backend is available.",
    );
}

fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<GetArgs>(scope, &args) else {
        return;
    };
    if !check_fully_active(scope) || check_aborted(scope, parsed.options.0.signal) {
        return;
    }
    backend_unavailable(scope);
}

fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<CreateArgs>(scope, &args) else {
        return;
    };
    if !check_fully_active(scope) {
        return;
    }
    let options = parsed.options.0;
    // Unlike get(), create() checks multiple credential types before abort.
    let types = usize::from(options.password.is_some())
        + usize::from(options.federated.is_some())
        + usize::from(options.public_key.is_some());
    if types > 1 {
        backend_unavailable(scope);
        return;
    }
    if !check_aborted(scope, options.signal) {
        backend_unavailable(scope);
    }
}

fn store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    if webidl::parse_args::<StoreArgs>(scope, &args).is_some() && check_fully_active(scope) {
        backend_unavailable(scope);
    }
}

fn prevent_silent_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !check_fully_active(scope) {
        return;
    }
    // The default prevent-silent-access flag is true. With no credential store
    // or automatic sign-in path, this cannot enable silent access or persist a
    // credential; its idempotent operation can complete normally.
    let value = v8::undefined(scope).into();
    crate::context_bootstrap::stream_adapter::set_resolved_promise(scope, &mut rv, value);
}
