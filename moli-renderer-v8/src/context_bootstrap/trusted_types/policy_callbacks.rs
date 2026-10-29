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
    util::{get_private_value, v8str},
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
    webidl,
    webidl_callback_execution::{PreparedWebIdlCallbackExecution, WebIdlCallbackExecutionOutcome},
};

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TrustedTypePolicyOptions", snapshot)]
pub(super) struct TrustedTypePolicyOptionsMembers {
    #[webidl(name = "createHTML", converter = "callback_function")]
    #[webidl(snapshot_slot = TRUSTED_TYPES_CREATE_HTML_SLOT, snapshot_state_with = callback_carrier)]
    create_html: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "createScript", converter = "callback_function")]
    #[webidl(snapshot_slot = TRUSTED_TYPES_CREATE_SCRIPT_SLOT, snapshot_state_with = callback_carrier)]
    create_script: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "createScriptURL", converter = "callback_function")]
    #[webidl(snapshot_slot = TRUSTED_TYPES_CREATE_SCRIPT_URL_SLOT, snapshot_state_with = callback_carrier)]
    create_script_url: Option<webidl::WebIdlCallbackFunction>,
}

fn callback_carrier<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: &webidl::WebIdlCallbackFunction,
) -> Option<v8::Local<'s, v8::Value>> {
    Some(
        V8TracedWebIdlCallbackFunction::from_callback(scope, callback)
            .into_object()
            .into(),
    )
}

/// Converts the optional `TrustedTypePolicyOptions` dictionary before the
/// Trusted Types policy-creation algorithm observes or mutates policy state.
///
/// Web IDL treats omitted, `undefined`, and `null` as an empty dictionary.
/// Member reads and callback conversion retain their ordinary getter order and
/// abrupt completion.
pub(super) fn parse_policy_callbacks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
) -> Option<TrustedTypePolicyOptionsMembers> {
    let members = match webidl::parse_dictionary::<TrustedTypePolicyOptionsMembers>(
        scope,
        options,
        webidl::Context::argument("TrustedTypePolicyFactory.createPolicy", 2),
    ) {
        Ok(Some(members)) => members,
        Ok(None) => TrustedTypePolicyOptionsMembers::default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    Some(members)
}

pub(super) enum TrustedTypePolicyCallbackOutcome {
    Missing,
    Returned(Option<DomStringValue>),
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
) -> TrustedTypePolicyCallbackOutcome {
    let Some(carrier) = get_private_value(scope, policy, kind.callback_slot())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return TrustedTypePolicyCallbackOutcome::Missing;
    };
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
            invoke_and_convert_policy_callback(scope, callback, receiver, arguments, kind)
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

fn invoke_and_convert_policy_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
    kind: TrustedTypeKind,
) -> Result<Option<DomStringValue>, ()> {
    let result = callback.call(scope, receiver, arguments).ok_or(())?;
    if result.is_null() || result.is_undefined() {
        return Ok(None);
    }
    let context = webidl::Context::member("TrustedTypePolicy callback", "return value");
    let converted = match kind {
        TrustedTypeKind::Html | TrustedTypeKind::Script => {
            webidl::convert::<webidl::DomString16>(scope, result, context)
                .map(|value| DomStringValue::from_utf16(&value.0))
        }
        TrustedTypeKind::ScriptUrl => webidl::convert::<webidl::UsvString>(scope, result, context)
            .map(|value| DomStringValue::from(value.0)),
    };
    converted.map(Some).map_err(|error| {
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
