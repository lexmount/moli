//! Scope-local access to the runtime that owns an AbortSignal and its algorithms.

use crate::embedding::realm_runtime;
use moli_webidl as webidl;

pub struct AbortSignalOwner {
    pub is_aborted: for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>) -> bool,
    pub reason: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Value>>,
    pub abort: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Value>,
    ),
    pub register_algorithm: AbortAlgorithmOperation,
    pub unregister_algorithm: AbortAlgorithmOperation,
}

type AbortAlgorithmOperation = for<'s, 'i> fn(
    &mut v8::PinScope<'s, 'i>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Function>,
) -> bool;

/// A validated signal and the operations of its existing lifetime owner.
#[derive(Clone, Copy)]
pub struct ResolvedAbortSignal<'s> {
    signal: v8::Local<'s, v8::Object>,
    owner: &'static AbortSignalOwner,
}

impl<'s> webidl::WebIdlConverter<'s> for ResolvedAbortSignal<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        v8::Local::<v8::Object>::try_from(value)
            .ok()
            .and_then(|signal| Self::resolve(scope, signal))
            .ok_or_else(|| webidl::WebIdlError::cannot_convert(context, "AbortSignal"))
    }
}

impl<'s> ResolvedAbortSignal<'s> {
    pub fn resolve(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let owner = (realm_runtime(scope)?.abort_signal_owner)(scope, signal)?;
        Some(Self { signal, owner })
    }

    pub fn value(self) -> v8::Local<'s, v8::Object> {
        self.signal
    }

    pub fn is_aborted(self, scope: &mut v8::PinScope<'s, '_>) -> bool {
        (self.owner.is_aborted)(scope, self.signal)
    }

    pub fn reason(self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        (self.owner.reason)(scope, self.signal)
    }

    pub fn abort(self, scope: &mut v8::PinScope<'s, '_>, reason: v8::Local<'s, v8::Value>) {
        (self.owner.abort)(scope, self.signal, reason);
    }

    pub fn register_algorithm(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        algorithm: v8::Local<'s, v8::Function>,
    ) -> bool {
        (self.owner.register_algorithm)(scope, self.signal, algorithm)
    }

    pub fn unregister_algorithm(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        algorithm: v8::Local<'s, v8::Function>,
    ) -> bool {
        (self.owner.unregister_algorithm)(scope, self.signal, algorithm)
    }
}
