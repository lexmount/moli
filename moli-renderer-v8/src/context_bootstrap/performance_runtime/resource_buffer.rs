use super::*;
use crate::{
    native_bridge::{JsContextHost, ResourceTimingBufferId},
    util::{
        context_host_ptr_from_global_bridge, get_private_value, set_private_value, throw_type_error,
    },
    webidl,
};
use moli_webapi_declare::WebApiFunctionTemplate;

const RESOURCE_TIMING_BUFFER_FULL_EVENT: &str = "resourcetimingbufferfull";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.setResourceTimingBufferSize")]
struct SetResourceTimingBufferSizeArgs {
    #[webidl(required)]
    max_size: u32,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Performance", enumerable)]
struct PerformanceResourceTimingBufferMembersDeclaration {
    #[webapi(method, length = 0, callback = clear_resource_timings_callback)]
    clear_resource_timings: (),

    #[webapi(method, length = 1, callback = set_resource_timing_buffer_size_callback)]
    set_resource_timing_buffer_size: (),

    #[webapi(
        accessor_property,
        getter = on_resource_timing_buffer_full_getter,
        setter = on_resource_timing_buffer_full_setter,
        enumerable
    )]
    onresourcetimingbufferfull: (),
}

pub(super) fn install_resource_timing_buffer_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    PerformanceResourceTimingBufferMembersDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(super) fn initialize_resource_timing_buffer_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) {
    if resource_timing_buffer_context(scope, performance).is_some() {
        return;
    }
    let host_ptr = context_host_ptr_from_global_bridge(scope)
        .expect("Performance must be initialized in a renderer context");
    let (buffer_id, finalizer) = unsafe { &mut *host_ptr }
        .create_resource_timing_buffer(scope, DEFAULT_RESOURCE_TIMING_BUFFER_SIZE);
    let id = v8::BigInt::new_from_u64(scope, buffer_id.raw());
    set_private_value(
        scope,
        performance,
        PERFORMANCE_RESOURCE_TIMING_BUFFER_ID_SLOT,
        id.into(),
    );
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, performance, move || {
        finalizer.finalize();
    });
}

pub(super) fn add_resource_timing_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    initialize_resource_timing_buffer_state(scope, performance);
    let Some((host_ptr, buffer_id)) = resource_timing_buffer_context(scope, performance) else {
        return;
    };
    if unsafe { &*host_ptr }.resource_timing_buffer_can_add_immediately(buffer_id) {
        append_resource_timing_to_primary(scope, host_ptr, performance, entry, buffer_id);
        return;
    }

    if unsafe { &*host_ptr }.mark_resource_timing_buffer_full_task_pending(buffer_id) {
        queue_resource_timing_buffer_full_task(scope, host_ptr, performance, buffer_id);
    }
    unsafe { &*host_ptr }
        .push_secondary_resource_timing_entry(buffer_id, v8::Global::new(scope, entry));
}

fn clear_resource_timings_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(performance) = performance_receiver(scope, &args) else {
        return;
    };
    // Resource Timing §3.4 intentionally clears only the primary buffer and
    // its current size here. The pending flag and secondary buffer survive so
    // the already-queued buffer-full task can process overflow entries; see
    // WPT resource-timing/buffer-full-add-then-clear.html.
    clear_primary_resource_timings(scope, performance);
    rv.set_undefined();
}

fn set_resource_timing_buffer_size_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(performance) = performance_receiver(scope, &args) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<SetResourceTimingBufferSizeArgs>(scope, &args) else {
        return;
    };
    initialize_resource_timing_buffer_state(scope, performance);
    if let Some((host_ptr, buffer_id)) = resource_timing_buffer_context(scope, performance) {
        unsafe { &*host_ptr }.set_resource_timing_buffer_size_limit(buffer_id, parsed.max_size);
    }
    rv.set_undefined();
}

fn on_resource_timing_buffer_full_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(performance) = performance_receiver(scope, &args) else {
        return;
    };
    let handler = get_private_value(
        scope,
        performance,
        PERFORMANCE_ON_RESOURCE_TIMING_BUFFER_FULL_SLOT,
    )
    .filter(|value| value.is_function())
    .unwrap_or_else(|| v8::null(scope).into());
    rv.set(handler);
}

fn on_resource_timing_buffer_full_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(performance) = performance_receiver(scope, &args) else {
        return;
    };
    let value = args.get(0);
    let handler = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(
        scope,
        performance,
        PERFORMANCE_ON_RESOURCE_TIMING_BUFFER_FULL_SLOT,
        handler,
    );
    simple_object_event_set_ordered_handler(
        scope,
        performance,
        PERFORMANCE_EVENT_LISTENERS_SLOT,
        RESOURCE_TIMING_BUFFER_FULL_EVENT,
        PERFORMANCE_ON_RESOURCE_TIMING_BUFFER_FULL_SLOT,
        handler.is_function(),
    );
}

fn performance_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    if performance_slot_value(scope, receiver, PERFORMANCE_TIME_ORIGIN_SLOT).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return None;
    }
    Some(receiver)
}

fn resource_timing_buffer_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, ResourceTimingBufferId)> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let value = get_private_value(
        scope,
        performance,
        PERFORMANCE_RESOURCE_TIMING_BUFFER_ID_SLOT,
    )?;
    let id = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (raw, lossless) = id.u64_value();
    lossless
        .then(|| ResourceTimingBufferId::from_raw(raw))
        .flatten()
        .map(|id| (host_ptr, id))
}

fn clear_primary_resource_timings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) {
    let Some(entries) = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT) else {
        return;
    };
    let retained = v8::Array::new(scope, 0);
    for index in 0..entries.length() {
        let Some(value) = entries.get_index(scope, index) else {
            continue;
        };
        let is_resource = v8::Local::<v8::Object>::try_from(value)
            .ok()
            .and_then(|entry| {
                performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT)
            })
            .as_deref()
            == Some("resource");
        if !is_resource {
            let _ = retained.set_index(scope, retained.length(), value);
        }
    }
    set_performance_slot_value(
        scope,
        performance,
        PERFORMANCE_ENTRIES_SLOT,
        retained.into(),
    );
    if let Some((host_ptr, buffer_id)) = resource_timing_buffer_context(scope, performance) {
        unsafe { &*host_ptr }.clear_resource_timing_primary_buffer(buffer_id);
    }
}

fn append_resource_timing_to_primary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
    buffer_id: ResourceTimingBufferId,
) {
    if performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT).is_none() {
        return;
    }
    super::entries::append_performance_entry(scope, performance, entry);
    unsafe { &*host_ptr }.note_resource_timing_primary_entry_added(buffer_id);
}

fn queue_resource_timing_buffer_full_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    performance: v8::Local<'s, v8::Object>,
    buffer_id: ResourceTimingBufferId,
) {
    let context = scope.get_current_context();
    let _ = unsafe { &mut *host_ptr }.queue_resource_timing_buffer_full_task(
        scope,
        context,
        performance,
        buffer_id,
    );
}

pub(crate) fn run_resource_timing_buffer_full_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    buffer_id: ResourceTimingBufferId,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    while let before @ 1.. = unsafe { &*host_ptr }.secondary_resource_timing_buffer_len(buffer_id) {
        if !unsafe { &*host_ptr }.resource_timing_buffer_can_add_to_primary(buffer_id) {
            dispatch_resource_timing_buffer_full_event(scope, performance);
        }
        copy_secondary_resource_timing_buffer(scope, host_ptr, performance, buffer_id);
        let after = unsafe { &*host_ptr }.secondary_resource_timing_buffer_len(buffer_id);
        if after >= before {
            unsafe { &*host_ptr }.clear_secondary_resource_timing_buffer(buffer_id);
            break;
        }
    }
    unsafe { &*host_ptr }.finish_resource_timing_buffer_full_task(buffer_id);
}

fn dispatch_resource_timing_buffer_full_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) {
    let global = scope.get_current_context().global(scope);
    let Some(event_constructor) = global
        .get(scope, v8str(scope, "Event").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let Some(event) = event_constructor.new_instance(
        scope,
        &[v8str(scope, RESOURCE_TIMING_BUFFER_FULL_EVENT).into()],
    ) else {
        return;
    };
    mark_event_trusted(scope, event);
    let _ = dispatch_simple_event_target_event(
        scope,
        performance,
        PERFORMANCE_EVENT_LISTENERS_SLOT,
        RESOURCE_TIMING_BUFFER_FULL_EVENT,
        event,
    );
}

fn copy_secondary_resource_timing_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    performance: v8::Local<'s, v8::Object>,
    buffer_id: ResourceTimingBufferId,
) {
    while unsafe { &*host_ptr }.resource_timing_buffer_can_add_to_primary(buffer_id) {
        let Some(entry) = unsafe { &*host_ptr }.pop_secondary_resource_timing_entry(buffer_id)
        else {
            break;
        };
        let entry = v8::Local::new(scope, &entry);
        append_resource_timing_to_primary(scope, host_ptr, performance, entry, buffer_id);
    }
}
