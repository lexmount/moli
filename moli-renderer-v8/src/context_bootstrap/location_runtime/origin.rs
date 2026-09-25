use super::access::location_target;
use crate::native_bridge::{WindowSecurityOrigin, window_contexts_allow_access};
use crate::util::context_host_ptr_from_context_slot;

const SECURITY_ORIGIN_FIELD: usize = 0;
pub(super) const INTERNAL_FIELD_COUNT: usize = 1;

pub(super) fn install<'s>(scope: &mut v8::PinScope<'s, '_>, location: v8::Local<'s, v8::Object>) {
    if for_target(scope, location).is_some() {
        return;
    }
    let owner = super::super::navigation_window::runtime_window_owner(scope, location);
    let origin = if let Some(popup_id) =
        crate::native_bridge::lightweight_popup_id_from_window(scope, owner)
    {
        context_host_ptr_from_context_slot(scope.get_current_context())
            .and_then(|host| unsafe { &*host }.lightweight_popup_security_origin(popup_id))
    } else {
        location
            .get_creation_context(scope)
            .and_then(WindowSecurityOrigin::for_context)
    };
    let Some(origin) = origin else {
        return;
    };
    let mut origin = Box::new(origin);
    let pointer = (&mut *origin as *mut WindowSecurityOrigin).cast();
    let value = v8::External::new(scope, pointer);
    location.set_internal_field(SECURITY_ORIGIN_FIELD, value.into());
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, location, move || drop(origin));
}

pub(super) fn refresh_from_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
) {
    let location = location_target(scope, location);
    let Some(origin) = WindowSecurityOrigin::for_context(scope.get_current_context()) else {
        return;
    };
    let Some(value) = location.get_internal_field(scope, SECURITY_ORIGIN_FIELD) else {
        return;
    };
    let Ok(external) = v8::Local::<v8::External>::try_from(value) else {
        return;
    };
    // The finalizer owns this Box; replacing its value preserves the Location
    // object's stable identity while updating its access-check snapshot.
    *unsafe { &mut *external.value().cast::<WindowSecurityOrigin>() } = origin;
}

fn for_target(
    scope: &mut v8::PinScope<'_, '_>,
    target: v8::Local<'_, v8::Object>,
) -> Option<WindowSecurityOrigin> {
    let value = target.get_internal_field(scope, SECURITY_ORIGIN_FIELD)?;
    let external = v8::Local::<v8::External>::try_from(value).ok()?;
    // Only native Location templates have this field. Its allocation is owned
    // by the target's finalizer, including through retained accessor functions.
    Some(unsafe { &*external.value().cast::<WindowSecurityOrigin>() }.clone())
}

pub(super) fn context_can_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    // Access-check callbacks cannot read private properties on their checked
    // receiver: that would recursively invoke this very access check.
    if let Some(target_origin) = for_target(scope, target) {
        let source = context_host_ptr_from_context_slot(context)
            .and_then(|host| unsafe { &*host }.window_security_origin_for_context(scope, context));
        return source.is_some_and(|source| source.can_access(&target_origin));
    }
    // Bootstrap can precede registration of the initial Window's origin.
    target
        .get_creation_context(scope)
        .is_some_and(|owner| window_contexts_allow_access(context, owner))
}

pub(super) fn callback_can_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callee: v8::Local<'s, v8::Object>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    let receiver = location_target(scope, receiver);
    if let (Some(source), Some(target)) = (for_target(scope, callee), for_target(scope, receiver)) {
        return source.can_access(&target);
    }
    receiver
        .get_creation_context(scope)
        .is_some_and(|owner| window_contexts_allow_access(scope.get_current_context(), owner))
}
