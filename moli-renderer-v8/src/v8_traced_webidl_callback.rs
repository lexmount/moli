//! V8-traced residence for Web IDL callback functions and interfaces.
//!
//! Some Web API owners already live as JavaScript objects. Storing their
//! callbacks in independent Rust `Global` roots would keep callback-to-owner
//! cycles alive after the owner becomes unreachable. This carrier keeps the
//! callback object and its two Web IDL context anchors in private V8 slots on
//! one owner-held object, then creates a temporary rooted snapshot only for an
//! invocation.
//!
//! This module owns no Window, worker, task, exception, or return-value policy.
//! Owners that need exact Window currentness must add that authorization before
//! invoking the prepared callback.

use moli_webidl_callback::{
    PreparedWebIdlCallbackFunction, PreparedWebIdlCallbackInterface, WebIdlCallbackFunction,
    WebIdlCallbackInterface,
};

use crate::util::{get_private_value, set_private_value};

const CALLBACK_VALUE_SLOT: &str = "__lmV8TracedWebIdlCallbackValue";
const CALLBACK_RELEVANT_GLOBAL_SLOT: &str = "__lmV8TracedWebIdlCallbackRelevantGlobal";
const CALLBACK_INCUMBENT_GLOBAL_SLOT: &str = "__lmV8TracedWebIdlCallbackIncumbentGlobal";
const CALLBACK_CALLABLE_SLOT: &str = "__lmV8TracedWebIdlCallbackCallableAtConversion";

/// Callback-interface residence owned by a JavaScript wrapper. Context extras
/// retain the original realms even after their WindowProxies are detached.
pub(crate) struct V8TracedWebIdlCallbackInterface<'s> {
    carrier: v8::Local<'s, v8::Object>,
}

impl<'s> V8TracedWebIdlCallbackInterface<'s> {
    pub(crate) fn new(scope: &mut v8::PinScope<'s, '_>, callback: WebIdlCallbackInterface) -> Self {
        let carrier = v8::Object::new(scope);
        let value = callback.value(scope);
        let relevant = callback
            .relevant_context(scope)
            .get_extras_binding_object(scope);
        let incumbent = callback
            .incumbent_context(scope)
            .get_extras_binding_object(scope);
        let callable = v8::Boolean::new(scope, callback.callable_at_conversion());
        set_private_value(scope, carrier, CALLBACK_VALUE_SLOT, value);
        set_private_value(
            scope,
            carrier,
            CALLBACK_RELEVANT_GLOBAL_SLOT,
            relevant.into(),
        );
        set_private_value(
            scope,
            carrier,
            CALLBACK_INCUMBENT_GLOBAL_SLOT,
            incumbent.into(),
        );
        set_private_value(scope, carrier, CALLBACK_CALLABLE_SLOT, callable.into());
        Self { carrier }
    }

    pub(crate) const fn from_object(carrier: v8::Local<'s, v8::Object>) -> Self {
        Self { carrier }
    }

    pub(crate) const fn into_object(self) -> v8::Local<'s, v8::Object> {
        self.carrier
    }

    pub(crate) fn prepare(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> PreparedWebIdlCallbackInterface {
        let object = |scope: &mut v8::PinScope<'s, '_>, slot| {
            get_private_value(scope, self.carrier, slot)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .expect("a traced callback interface must retain its registration facts")
        };
        let callback = object(scope, CALLBACK_VALUE_SLOT);
        let relevant_context = object(scope, CALLBACK_RELEVANT_GLOBAL_SLOT)
            .get_creation_context(scope)
            .expect("callback relevant realm anchor");
        let incumbent_context = object(scope, CALLBACK_INCUMBENT_GLOBAL_SLOT)
            .get_creation_context(scope)
            .expect("callback incumbent realm anchor");
        let callable = get_private_value(scope, self.carrier, CALLBACK_CALLABLE_SLOT)
            .expect("callback-interface callability at conversion")
            .is_true();
        PreparedWebIdlCallbackInterface::new(
            scope,
            callback,
            relevant_context,
            incumbent_context,
            callable,
        )
    }
}

pub(crate) struct V8TracedWebIdlCallbackFunction<'s> {
    carrier: v8::Local<'s, v8::Object>,
}

impl<'s> V8TracedWebIdlCallbackFunction<'s> {
    pub(crate) fn new(scope: &mut v8::PinScope<'s, '_>, callback: WebIdlCallbackFunction) -> Self {
        let carrier = v8::Object::new(scope);
        let callback_value = v8::Local::<v8::Object>::try_from(callback.value(scope))
            .expect("a Web IDL callback function must be a callable object");
        let relevant_global = callback.relevant_context(scope).global(scope);
        let incumbent_global = callback.incumbent_context(scope).global(scope);
        set_private_value(scope, carrier, CALLBACK_VALUE_SLOT, callback_value.into());
        set_private_value(
            scope,
            carrier,
            CALLBACK_RELEVANT_GLOBAL_SLOT,
            relevant_global.into(),
        );
        set_private_value(
            scope,
            carrier,
            CALLBACK_INCUMBENT_GLOBAL_SLOT,
            incumbent_global.into(),
        );
        Self { carrier }
    }

    pub(crate) const fn from_object(carrier: v8::Local<'s, v8::Object>) -> Self {
        Self { carrier }
    }

    pub(crate) const fn into_object(self) -> v8::Local<'s, v8::Object> {
        self.carrier
    }

    pub(crate) fn prepare(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> PreparedWebIdlCallbackFunction {
        let callback = get_private_value(scope, self.carrier, CALLBACK_VALUE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("a traced callback residence must retain its callback object");
        let relevant_global = get_private_value(scope, self.carrier, CALLBACK_RELEVANT_GLOBAL_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("a traced callback residence must retain its relevant global");
        let incumbent_global =
            get_private_value(scope, self.carrier, CALLBACK_INCUMBENT_GLOBAL_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .expect("a traced callback residence must retain its incumbent global");
        let relevant_context = relevant_global
            .get_creation_context(scope)
            .expect("a traced callback relevant global must retain its context");
        let incumbent_context = incumbent_global
            .get_creation_context(scope)
            .expect("a traced callback incumbent global must retain its context");
        PreparedWebIdlCallbackFunction::try_new(
            scope,
            callback,
            relevant_context,
            incumbent_context,
        )
        .expect("a traced Web IDL callback function must remain callable")
    }
}
