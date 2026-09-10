mod bindings;
mod methods;
mod store;

use super::*;

pub(crate) use self::bindings::headers_constructor_callback;
pub(super) use self::methods::install_headers_object_methods;
pub(crate) use self::methods::install_headers_template_bindings;
pub(crate) use self::store::headers_entries;
pub(crate) use self::store::{HeadersGuard, filter_headers_for_guard};
pub(super) use self::store::{
    build_headers_object, build_headers_object_with_state, headers_entries_from_init,
    mark_headers_immutable,
};

pub(super) fn clone_headers_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let entries = headers_entries(scope, original);
    let guard = self::store::headers_guard(scope, original);
    let immutable = self::store::headers_are_immutable(scope, original);
    let clone = build_headers_object_with_state(scope, &entries, guard, immutable);
    if let Some(prototype) = moli_v8_util::global_constructor_prototype(scope, "Headers") {
        let _ = clone.set_prototype(scope, prototype.into());
    }
    install_headers_object_methods(scope, clone);
    clone
}
