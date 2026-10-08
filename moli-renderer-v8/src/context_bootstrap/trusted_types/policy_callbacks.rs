//! Typed Web IDL callback ownership for `TrustedTypePolicy`.
//!
//! The policy object remains the sole callback residence. Each supplied
//! dictionary member is converted once, then stored as a V8-traced carrier in
//! the exact policy's private slot. Invocation enters the callback's relevant
//! and conversion-time incumbent contexts, while this module leaves policy
//! naming, CSP, Trusted Type construction, sink selection, and default-policy
//! result handling to `trusted_types`.

use moli_webidl_callback::invoke_webidl_callback_function;

use super::TrustedTypeKind;
use crate::dom::native::DomStringValue;
use crate::{
    util::{context_host_ptr_from_global_bridge, get_private_value, v8str},
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
    webidl,
    window_webidl_callback::{
        PreparedWindowWebIdlCallbackFunctionOutcome, V8TracedWindowWebIdlCallbackFunction,
    },
};

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TrustedTypePolicyOptions")]
struct TrustedTypePolicyOptionsMembers {
    #[webidl(name = "createHTML", converter = "callback_function")]
    create_html: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "createScript", converter = "callback_function")]
    create_script: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "createScriptURL", converter = "callback_function")]
    create_script_url: Option<webidl::WebIdlCallbackFunction>,
}

pub(super) struct TrustedTypePolicyCallbackCarriers<'s> {
    pub(super) create_html: Option<v8::Local<'s, v8::Object>>,
    pub(super) create_script: Option<v8::Local<'s, v8::Object>>,
    pub(super) create_script_url: Option<v8::Local<'s, v8::Object>>,
}

impl<'s> TrustedTypePolicyCallbackCarriers<'s> {
    fn from_members(
        scope: &mut v8::PinScope<'s, '_>,
        members: TrustedTypePolicyOptionsMembers,
    ) -> Self {
        Self {
            create_html: callback_carrier(scope, members.create_html),
            create_script: callback_carrier(scope, members.create_script),
            create_script_url: callback_carrier(scope, members.create_script_url),
        }
    }
}

fn callback_carrier<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: Option<webidl::WebIdlCallbackFunction>,
) -> Option<v8::Local<'s, v8::Object>> {
    callback.map(|callback| V8TracedWebIdlCallbackFunction::new(scope, callback).into_object())
}

/// Converts the optional `TrustedTypePolicyOptions` dictionary before the
/// Trusted Types policy-creation algorithm observes or mutates policy state.
///
/// Web IDL treats omitted, `undefined`, and `null` as an empty dictionary.
/// Member reads and callback conversion retain their ordinary getter order and
/// abrupt completion.
pub(super) fn parse_policy_callback_carriers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
) -> Option<TrustedTypePolicyCallbackCarriers<'s>> {
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
    Some(TrustedTypePolicyCallbackCarriers::from_members(
        scope, members,
    ))
}

pub(super) enum TrustedTypePolicyCallbackOutcome {
    Missing,
    Returned(Option<DomStringValue>),
    Abrupt,
}

/// Invokes one policy-owned callback and performs its nullable string return
/// conversion before leaving the callback's relevant Realm.
///
/// Window callbacks additionally require their exact callback Realm to remain
/// current. Worker callbacks share one isolate/run lifetime with the policy and
/// therefore need no generation or current-worker lookup.
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

    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        let callback =
            V8TracedWindowWebIdlCallbackFunction::from_object(carrier).prepare(scope, host);
        return match callback.invoke(
            scope,
            host,
            receiver,
            arguments,
            |scope, callback, receiver, arguments| {
                invoke_and_convert_policy_callback(scope, callback, receiver, arguments, kind)
            },
        ) {
            PreparedWindowWebIdlCallbackFunctionOutcome::Returned(value) => {
                TrustedTypePolicyCallbackOutcome::Returned(value)
            }
            PreparedWindowWebIdlCallbackFunctionOutcome::Failed(()) => {
                TrustedTypePolicyCallbackOutcome::Abrupt
            }
            PreparedWindowWebIdlCallbackFunctionOutcome::Retired => {
                throw_callback_no_longer_runnable(scope);
                TrustedTypePolicyCallbackOutcome::Abrupt
            }
        };
    }

    if crate::worker::get_worker_state(scope).is_none() {
        throw_callback_no_longer_runnable(scope);
        return TrustedTypePolicyCallbackOutcome::Abrupt;
    }
    let callback = V8TracedWebIdlCallbackFunction::from_object(carrier).prepare(scope);
    match invoke_webidl_callback_function(
        scope,
        &callback,
        receiver,
        arguments,
        |scope, callback, receiver, arguments| {
            invoke_and_convert_policy_callback(scope, callback, receiver, arguments, kind)
        },
    ) {
        Ok(value) => TrustedTypePolicyCallbackOutcome::Returned(value),
        Err(()) => TrustedTypePolicyCallbackOutcome::Abrupt,
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
