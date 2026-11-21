//! Native capabilities supplied by the runtime for a realm.

use std::rc::Rc;

pub type WebIdlCallbackResult<'s> =
    Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>>;
pub type WebIdlCallbackInvocation<'s> = for<'i> fn(
    &mut v8::PinScope<'s, 'i>,
    v8::Local<'s, v8::Function>,
    v8::Local<'s, v8::Value>,
    &[v8::Local<'s, v8::Value>],
) -> WebIdlCallbackResult<'s>;

/// Bridges value bindings to the embedder without retaining its host or V8 handles.
/// Callbacks execute in the current (callee) realm, including for borrowed methods.
pub struct RealmCallbacks {
    /// Writes a live SVG point back to its owner; ordinary points need no action.
    pub point_mutated: for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>),
    /// Reads the native source of a live MediaQueryListEvent without invoking script.
    pub media_query_matches:
        for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>) -> Option<bool>,
    pub runtime: Option<&'static RealmRuntime>,
}

/// Capabilities needed by bindings that interact with the owning execution context.
pub struct RealmRuntime {
    pub abort_signal_owner:
        for<'s, 'i> fn(
            &mut v8::PinScope<'s, 'i>,
            v8::Local<'s, v8::Object>,
        ) -> Option<&'static crate::abort_signal::AbortSignalOwner>,
    pub structured_clone_value: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Value>,
    ) -> Option<v8::Local<'s, v8::Value>>,
    pub owner_terminating: fn(&mut v8::PinScope<'_, '_>) -> bool,
    pub trace_callback: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        moli_webidl::WebIdlCallbackFunction,
    ) -> v8::Local<'s, v8::Object>,
    pub invoke_traced_callback: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Value>,
        &[v8::Local<'s, v8::Value>],
        WebIdlCallbackInvocation<'s>,
    ) -> Option<WebIdlCallbackResult<'s>>,
    pub blob: crate::blob::BlobRuntime,
    pub form_data: crate::form_data::FormDataRuntime,
}

/// Installs the runtime's native capabilities before author code can use this realm.
pub fn install_realm_callbacks(scope: &mut v8::PinScope<'_, '_>, callbacks: RealmCallbacks) {
    scope.get_current_context().set_slot(Rc::new(callbacks));
}

pub(crate) fn realm_callbacks(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<RealmCallbacks>> {
    scope.get_current_context().get_slot::<RealmCallbacks>()
}

pub(crate) fn realm_runtime(scope: &mut v8::PinScope<'_, '_>) -> Option<&'static RealmRuntime> {
    realm_callbacks(scope)?.runtime
}

pub(crate) fn throw_dom_exception_value(
    scope: &mut v8::PinScope<'_, '_>,
    message: &str,
    name: &'static str,
) {
    let exception =
        crate::dom_exception::new_most_derived_dom_exception_value(scope, message, name);
    scope.throw_exception(exception);
}

pub(crate) fn reflect_point_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: v8::Local<'s, v8::Object>,
) {
    if let Some(callbacks) = realm_callbacks(scope) {
        (callbacks.point_mutated)(scope, point);
    }
}

pub(crate) fn read_media_query_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    let callbacks = realm_callbacks(scope)?;
    (callbacks.media_query_matches)(scope, list)
}
