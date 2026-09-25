use super::location_navigation::{
    LocationNavigationKind, navigate_location_object,
    navigate_location_object_with_child_navigate_event,
};
use super::navigation_callbacks::{document_location_getter, document_location_setter};
use super::*;

mod access;
mod helpers;
mod install;
mod methods;
mod navigation;
mod origin;
mod slots;
mod surface;

pub(in crate::context_bootstrap) use access::wrap_location_object;

pub(super) use install::{
    build_location_constructor_template, build_location_runtime_object,
    install_location_runtime_state, location_belongs_to_current_local_window,
    location_has_relevant_document, location_owner_has_current_realm,
};
pub(super) use navigation::{
    is_same_document_fragment_navigation, resolve_location_navigation_target,
};
pub(super) use slots::{location_href_slot, sync_location_object};
pub(crate) use surface::sync_global_location_runtime_state;
pub(crate) use surface::{
    install_constructed_document_location_runtime_state,
    sync_document_location_runtime_state_from_window,
    sync_window_location_history_navigation_runtime_surface, sync_window_location_runtime_state,
    window_location_href,
};
pub(in crate::context_bootstrap) use surface::{window_location_setter, window_navigation_setter};

pub(in crate::context_bootstrap) fn put_forward_location_href<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    // [PutForwards=href] gets the receiver's Location and assigns the original
    // value. Its href setter owns conversion, the exception realm, and navigation.
    let Some(location) = receiver.get(scope, v8str(scope, "location").into()) else {
        return;
    };
    let Ok(location) = v8::Local::<v8::Object>::try_from(location) else {
        crate::webidl::throw_type_error(scope, "Cannot assign href to a null Location.");
        return;
    };
    let _ = access::set_location_href(scope, location, value);
}
