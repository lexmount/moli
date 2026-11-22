//! Typed Web IDL callback ownership for `TrustedTypePolicy`.
//!
//! The policy object remains the sole callback residence. Each supplied
//! dictionary member is converted once, then stored as a V8-traced carrier in
//! the exact policy's private slot. Invocation enters the callback's relevant
//! and conversion-time incumbent contexts, while this module leaves policy
//! naming, CSP, Trusted Type construction, sink selection, and default-policy
//! result handling to `trusted_types`.

use super::{
    TRUSTED_TYPES_CREATE_HTML_SLOT, TRUSTED_TYPES_CREATE_SCRIPT_SLOT,
    TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT, TrustedTypeKind,
};
use crate::dom::native::DomStringValue;
use crate::{
    util::{get_private_value, set_private_value, v8str},
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
    webidl,
    webidl_callback_execution::{PreparedWebIdlCallbackExecution, WebIdlCallbackExecutionOutcome},
};
use moli_webapi::trusted_values::TrustedTypePolicyOptionsMembers;

pub(super) const CREATE_PARSER_OPTIONS_SLOT: &str = "moli.TrustedTypePolicy.createParserOptions";

/// Retains the converted callbacks on their policy with the resource initiator.
pub(super) fn install_policy_callbacks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    policy: v8::Local<'s, v8::Object>,
    callbacks: TrustedTypePolicyOptionsMembers,
) {
    for (slot, callback) in [
        (TRUSTED_TYPES_CREATE_HTML_SLOT, callbacks.create_html),
        (CREATE_PARSER_OPTIONS_SLOT, callbacks.create_parser_options),
        (TRUSTED_TYPES_CREATE_SCRIPT_SLOT, callbacks.create_script),
        (
            TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT,
            callbacks.create_script_url,
        ),
    ] {
        if let Some(callback) = callback {
            let carrier = V8TracedWebIdlCallbackFunction::from_callback(scope, &callback);
            set_private_value(scope, policy, slot, carrier.into_object().into());
        }
    }
}

pub(super) enum TrustedTypePolicyCallbackOutcome<T> {
    Missing,
    Returned(Option<T>),
    Abrupt,
}

/// Invokes one policy-owned callback and performs its nullable string return
/// conversion before leaving the callback's relevant Realm.
///
/// The renderer callback boundary authorizes the exact Window callback Realm
/// or Worker run. The policy keeps synchronous exception and string-conversion
/// semantics, including conversion in the callback's relevant Realm.
pub(super) fn invoke_policy_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    policy: v8::Local<'s, v8::Object>,
    kind: TrustedTypeKind,
    arguments: &[v8::Local<'s, v8::Value>],
) -> TrustedTypePolicyCallbackOutcome<DomStringValue> {
    let Some(carrier) = policy_callback(scope, policy, kind.callback_slot()) else {
        return TrustedTypePolicyCallbackOutcome::Missing;
    };
    invoke_callback(scope, carrier, arguments, |scope, value| {
        convert_string(scope, value, kind)
    })
}

pub(super) fn policy_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    policy: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, policy, slot).and_then(|value| v8::Local::try_from(value).ok())
}

pub(super) fn invoke_callback<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    carrier: v8::Local<'s, v8::Object>,
    arguments: &[v8::Local<'s, v8::Value>],
    convert: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Value>) -> Result<T, ()>,
) -> TrustedTypePolicyCallbackOutcome<T> {
    let receiver = v8::undefined(scope).into();

    let Some(callback) = PreparedWebIdlCallbackExecution::from_traced(scope, carrier) else {
        throw_callback_no_longer_runnable(scope);
        return TrustedTypePolicyCallbackOutcome::Abrupt;
    };
    match callback.invoke_with(
        scope,
        receiver,
        arguments,
        |scope, callback, receiver, arguments| {
            let value = callback.call(scope, receiver, arguments).ok_or(())?;
            if value.is_null_or_undefined() {
                return Ok(None);
            }
            convert(scope, value).map(Some)
        },
    ) {
        WebIdlCallbackExecutionOutcome::Returned(value) => {
            TrustedTypePolicyCallbackOutcome::Returned(value)
        }
        WebIdlCallbackExecutionOutcome::Failed(()) => TrustedTypePolicyCallbackOutcome::Abrupt,
        WebIdlCallbackExecutionOutcome::Unavailable => {
            throw_callback_no_longer_runnable(scope);
            TrustedTypePolicyCallbackOutcome::Abrupt
        }
    }
}

fn convert_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: v8::Local<'s, v8::Value>,
    kind: TrustedTypeKind,
) -> Result<DomStringValue, ()> {
    let context = webidl::Context::member("TrustedTypePolicy callback", "return value");
    let converted = match kind {
        TrustedTypeKind::Html | TrustedTypeKind::Script => {
            webidl::convert::<webidl::DomString16>(scope, result, context)
                .map(|value| DomStringValue::from_utf16(&value.0))
        }
        TrustedTypeKind::ScriptUrl => webidl::convert::<webidl::UsvString>(scope, result, context)
            .map(|value| DomStringValue::from(value.0)),
    };
    converted.map_err(|error| {
        webidl::throw_error(scope, &error);
    })
}

fn throw_callback_no_longer_runnable(scope: &mut v8::PinScope<'_, '_>) {
    let message = v8str(
        scope,
        "Failed to execute TrustedTypePolicy callback: the provided callback is no longer runnable.",
    );
    scope.throw_exception(v8::Exception::error(scope, message));
}
