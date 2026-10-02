//! Project platform objects, not arbitrary JavaScript values, into a world.

use super::{
    abort_signal, abort_signal_events, form_data_runtime, shared_event_targets, world_wrappers,
};
use crate::web_api_interfaces;

pub(super) fn in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Some(value);
    };
    let object = world_wrappers::owner(scope, object);
    if world_wrappers::belongs_to_world(scope, object, context) {
        return Some(object.into());
    }
    if let Some(wrapper) = world_wrappers::get(scope, object, context) {
        return Some(wrapper.into());
    }
    if super::is_window_receiver(scope, object) {
        return Some(window_in_realm(scope, object, context).into());
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let wrapper = if web_api_interfaces::AbortSignal::is_instance(scope, object) {
        let wrapper = abort_signal::new_signal(scope)?;
        if !crate::native_bridge::abort::bind_signal_wrapper(scope, object, wrapper) {
            return None;
        }
        abort_signal_events::initialize(scope, wrapper);
        shared_event_targets::bind_shared_target(scope, object, object);
        shared_event_targets::bind_shared_target(scope, wrapper, object);
        wrapper
    } else if web_api_interfaces::FormData::is_instance(scope, object) {
        form_data_runtime::new_world_wrapper(scope)?
    } else if web_api_interfaces::Node::is_instance(scope, object) {
        let (host_ptr, handle) =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)
                .ok()?;
        return unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, handle)
            .map(Into::into);
    } else if web_api_interfaces::Blob::is_instance(scope, object) {
        let wrapper = crate::blob::world_wrapper(scope, object)?;
        if web_api_interfaces::File::is_instance(scope, object) {
            super::file_api::bind_file_world_wrapper(scope, object, wrapper)?;
        }
        wrapper
    } else {
        return Some(value);
    };
    world_wrappers::insert(scope, object, wrapper);
    Some(wrapper.into())
}

fn window_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Object> {
    let Some(source_context) = window.get_creation_context(scope) else {
        return window;
    };
    if !window.strict_equals(source_context.global(scope).into()) {
        return window;
    }
    let Some(host_ptr) = crate::util::context_host_ptr_from_context_slot(context) else {
        return window;
    };
    if crate::util::context_host_ptr_from_context_slot(source_context) != Some(host_ptr) {
        return window;
    }
    let host = unsafe { &*host_ptr };
    let source = host.window_execution_context_identity_for_access_check(source_context);
    let destination = host.window_execution_context_identity_for_access_check(context);
    if let (Some(source), Some(destination)) = (source, destination)
        && source.owner() == destination.owner()
        && host.window_execution_context_identity_is_current(source)
        && host.window_execution_context_identity_is_current(destination)
    {
        context.global(scope)
    } else {
        window
    }
}
