//! Native capabilities supplied by the runtime for a realm.

use std::rc::Rc;

/// Bridges value bindings to the embedder without retaining its host or V8 handles.
/// Callbacks execute in the current (callee) realm, including for borrowed methods.
pub struct RealmCallbacks {
    /// Writes a live SVG point back to its owner; ordinary points need no action.
    pub point_mutated: for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>),
}

/// Installs the runtime's native capabilities before author code can use this realm.
pub fn install_realm_callbacks(scope: &mut v8::PinScope<'_, '_>, callbacks: RealmCallbacks) {
    scope.get_current_context().set_slot(Rc::new(callbacks));
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
    if let Some(callbacks) = scope.get_current_context().get_slot::<RealmCallbacks>() {
        (callbacks.point_mutated)(scope, point);
    }
}
