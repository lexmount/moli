use super::*;
use crate::web_api_interfaces;
use crate::{
    native_bridge::{ResourceTimingBufferId, SharedResourceTimingBufferRegistry},
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value},
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
#[webapi(interface = web_api_interfaces::Performance, enumerable, receiver)]
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
    if resource_timing_buffer_id(scope, performance).is_some() {
        return;
    }
    let registry = resource_timing_buffer_registry(scope)
        .expect("Performance must belong to a page or worker context");
    let (buffer_id, finalizer) =
        registry.create_resource_timing_buffer(DEFAULT_RESOURCE_TIMING_BUFFER_SIZE);
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
    let Some((registry, buffer_id)) = resource_timing_buffer_context(scope, performance) else {
        return;
    };
    // Count a full primary buffer when the entry is queued, including
    // entries later rescued by a buffer-full handler.
    registry.note_resource_timing_entry_queued(buffer_id);
    if registry.resource_timing_buffer_can_add_immediately(buffer_id) {
        append_resource_timing_to_primary(scope, &registry, performance, entry, buffer_id);
        return;
    }

    if registry.mark_resource_timing_buffer_full_task_pending(buffer_id) {
        queue_resource_timing_buffer_full_task(scope, performance, buffer_id);
    }
    registry.push_secondary_resource_timing_entry(buffer_id, v8::Global::new(scope, entry));
}

pub(in crate::context_bootstrap) fn resource_timing_dropped_entries_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> u64 {
    resource_timing_buffer_registry(scope)
        .zip(resource_timing_buffer_id(scope, performance))
        .map_or(0, |(registry, id)| {
            registry.resource_timing_dropped_entries_count(id)
        })
}

fn clear_resource_timings_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let performance = args.this();
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
    let performance = args.this();
    let Some(parsed) = webidl::parse_args::<SetResourceTimingBufferSizeArgs>(scope, &args) else {
        return;
    };
    initialize_resource_timing_buffer_state(scope, performance);
    if let Some(registry) = resource_timing_buffer_registry(scope)
        && let Some(buffer_id) = resource_timing_buffer_id(scope, performance)
    {
        registry.set_resource_timing_buffer_size_limit(buffer_id, parsed.max_size);
    }
    rv.set_undefined();
}

fn on_resource_timing_buffer_full_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let performance = args.this();
    let handler = get_private_value(
        scope,
        performance,
        PERFORMANCE_ON_RESOURCE_TIMING_BUFFER_FULL_SLOT,
    )
    .filter(|value| value.is_object())
    .unwrap_or_else(|| v8::null(scope).into());
    rv.set(handler);
}

fn on_resource_timing_buffer_full_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let performance = args.this();
    simple_object_event_set_handler(
        scope,
        performance,
        RESOURCE_TIMING_BUFFER_FULL_EVENT,
        PERFORMANCE_ON_RESOURCE_TIMING_BUFFER_FULL_SLOT,
        args.get(0),
    );
}

fn resource_timing_buffer_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> Option<(SharedResourceTimingBufferRegistry, ResourceTimingBufferId)> {
    let registry = resource_timing_buffer_registry(scope)?;
    resource_timing_buffer_id(scope, performance).map(|id| (registry, id))
}

fn resource_timing_buffer_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<SharedResourceTimingBufferRegistry> {
    if let Some(state) = crate::worker::get_worker_state(scope) {
        return Some(state.borrow().resource_timing_buffers.clone());
    }
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    Some(unsafe { &*host_ptr }.resource_timing_buffer_registry())
}

fn resource_timing_buffer_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> Option<ResourceTimingBufferId> {
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
    if let Some(registry) = resource_timing_buffer_registry(scope)
        && let Some(buffer_id) = resource_timing_buffer_id(scope, performance)
    {
        registry.clear_resource_timing_primary_buffer(buffer_id);
    }
}

fn append_resource_timing_to_primary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &SharedResourceTimingBufferRegistry,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
    buffer_id: ResourceTimingBufferId,
) {
    if performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT).is_none() {
        return;
    }
    super::entries::append_performance_entry(scope, performance, entry);
    registry.note_resource_timing_primary_entry_added(buffer_id);
}

fn queue_resource_timing_buffer_full_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    buffer_id: ResourceTimingBufferId,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let context = scope.get_current_context();
        let _ = unsafe { &mut *host_ptr }.queue_resource_timing_buffer_full_task(
            scope,
            context,
            performance,
            buffer_id,
        );
        return;
    }
    let id = v8::BigInt::new_from_u64(scope, buffer_id.raw());
    let data = v8::Array::new_with_elements(scope, &[performance.into(), id.into()]);
    let callback = v8::Function::builder(worker_resource_timing_buffer_full_callback)
        .data(data.into())
        .build(scope)
        .expect("resource timing task callback should allocate");
    assert!(crate::worker::queue_worker_networking_task(scope, callback));
}

fn worker_resource_timing_buffer_full_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Array>::try_from(args.data())
        .expect("resource timing task must retain its native callback data");
    let performance = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("resource timing task must retain its Performance");
    let id = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .expect("resource timing task must retain its buffer id");
    let (raw, lossless) = id.u64_value();
    assert!(lossless);
    let id = ResourceTimingBufferId::from_raw(raw).expect("resource timing buffer id is nonzero");
    run_resource_timing_buffer_full_task(scope, performance, id);
}

pub(crate) fn run_resource_timing_buffer_full_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    buffer_id: ResourceTimingBufferId,
) {
    let Some(registry) = resource_timing_buffer_registry(scope) else {
        return;
    };
    while let before @ 1.. = registry.secondary_resource_timing_buffer_len(buffer_id) {
        if !registry.resource_timing_buffer_can_add_to_primary(buffer_id) {
            dispatch_resource_timing_buffer_full_event(scope, performance);
        }
        copy_secondary_resource_timing_buffer(scope, &registry, performance, buffer_id);
        let after = registry.secondary_resource_timing_buffer_len(buffer_id);
        if after >= before {
            registry.clear_secondary_resource_timing_buffer(buffer_id);
            break;
        }
    }
    registry.finish_resource_timing_buffer_full_task(buffer_id);
}

fn dispatch_resource_timing_buffer_full_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) {
    let Some(event) =
        super::super::events::construct_original_event(scope, RESOURCE_TIMING_BUFFER_FULL_EVENT)
    else {
        return;
    };
    let _ = dispatch_simple_event_target_event(
        scope,
        performance,
        RESOURCE_TIMING_BUFFER_FULL_EVENT,
        event,
    );
}

fn copy_secondary_resource_timing_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &SharedResourceTimingBufferRegistry,
    performance: v8::Local<'s, v8::Object>,
    buffer_id: ResourceTimingBufferId,
) {
    while registry.resource_timing_buffer_can_add_to_primary(buffer_id) {
        let Some(entry) = registry.pop_secondary_resource_timing_entry(buffer_id) else {
            break;
        };
        let entry = v8::Local::new(scope, &entry);
        append_resource_timing_to_primary(scope, registry, performance, entry, buffer_id);
    }
}
