use super::PERFORMANCE_TIME_ORIGIN_SLOT;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
    throw_type_error,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;
use std::time::{Duration, Instant};

const MEMORY_PROTOTYPE_SLOT: &str = "__moliPerformanceMemoryPrototype";
const TOTAL_HEAP_SLOT: &str = "__moliMemoryInfoTotalHeap";
const USED_HEAP_SLOT: &str = "__moliMemoryInfoUsedHeap";
const HEAP_LIMIT_SLOT: &str = "__moliMemoryInfoHeapLimit";
const MEMORY_SLOTS: &[&str] = &[TOTAL_HEAP_SLOT, USED_HEAP_SLOT, HEAP_LIMIT_SLOT];
const SAMPLE_INTERVAL: Duration = Duration::from_secs(20 * 60);

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct MemoryInfoPrototypeDeclaration {
    #[webapi(accessor_property, name = "totalJSHeapSize", enumerable, getter = memory_info_getter, data = callback_data_index_value(scope, 0))]
    total_js_heap_size: (),

    #[webapi(accessor_property, name = "usedJSHeapSize", enumerable, getter = memory_info_getter, data = callback_data_index_value(scope, 1))]
    used_js_heap_size: (),

    #[webapi(accessor_property, name = "jsHeapSizeLimit", enumerable, getter = memory_info_getter, data = callback_data_index_value(scope, 2))]
    js_heap_size_limit: (),

    #[webapi(to_string_tag, readonly, init = string("MemoryInfo"))]
    tag: (),
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::MemoryInfo)]
struct MemoryInfoObjectDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(slot = TOTAL_HEAP_SLOT)]
    total: f64,
    #[webapi(slot = USED_HEAP_SLOT)]
    used: f64,
    #[webapi(slot = HEAP_LIMIT_SLOT)]
    limit: f64,
}

#[derive(Clone, Copy)]
struct HeapSample {
    taken_at: Instant,
    sizes: [f64; 3],
}

pub(super) fn performance_memory_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if get_private_value(scope, args.this(), PERFORMANCE_TIME_ORIGIN_SLOT).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let sizes = heap_sample(scope);
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let prototype = match get_private_value(scope, args.this(), MEMORY_PROTOTYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        Some(prototype) => prototype,
        None => {
            // MemoryInfo is a legacy interface without a global constructor.
            let Ok(prototype) = MemoryInfoPrototypeDeclaration::default().bind(scope) else {
                return;
            };
            set_private_value(scope, args.this(), MEMORY_PROTOTYPE_SLOT, prototype.into());
            prototype
        }
    };
    if let Ok(snapshot) =
        MemoryInfoObjectDeclaration::new(Some(prototype), sizes[0], sizes[1], sizes[2]).bind(scope)
    {
        rv.set(snapshot.into());
    }
}

fn memory_info_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, MEMORY_SLOTS, "MemoryInfo attribute") else {
        return;
    };
    let Some(value) = get_private_value(scope, args.this(), slot) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

fn heap_sample(scope: &mut v8::PinScope<'_, '_>) -> [f64; 3] {
    let now = Instant::now();
    if let Some(sample) = scope.get_slot::<HeapSample>()
        && now.duration_since(sample.taken_at) < SAMPLE_INTERVAL
    {
        return sample.sizes;
    }
    let statistics = scope.get_heap_statistics();
    let external = statistics.external_memory();
    let sizes = [
        statistics.total_physical_size().saturating_add(external),
        statistics.used_heap_size().saturating_add(external),
        statistics.heap_size_limit(),
    ]
    .map(|size| quantize_heap_size(size) as f64);
    scope.set_slot(HeapSample {
        taken_at: now,
        sizes,
    });
    sizes
}

// Chromium's legacy API exposes coarse, rate-limited V8 statistics. Use 100
// exponentially spaced buckets from 10 MB towards 4 GB, rounded to three
// significant digits, instead of exposing fine-grained allocation/GC timing.
fn quantize_heap_size(size: usize) -> usize {
    let growth = (400.0_f32.ln() / 100.0).exp();
    let mut boundary = 10_000_000.0_f32;
    let mut decimal_threshold = 100_000_000_u64;
    let mut precision = 100_000_u64;
    let mut rounded = 0;
    for _ in 0..100 {
        rounded = (boundary as u64 / precision * precision) as usize;
        if size <= rounded {
            return rounded;
        }
        boundary *= growth;
        if boundary >= decimal_threshold as f32 {
            decimal_threshold *= 10;
            precision *= 10;
        }
    }
    rounded
}

#[cfg(test)]
mod tests {
    use super::quantize_heap_size;

    #[test]
    fn memory_heap_buckets_are_coarse_monotonic_and_bounded() {
        assert_eq!(quantize_heap_size(0), 10_000_000);
        assert_eq!(quantize_heap_size(10_000_000), 10_000_000);
        assert_eq!(quantize_heap_size(10_000_001), 10_600_000);
        let mut previous = 0;
        for size in (0..4_000_000_000_usize).step_by(1_000_000) {
            let bucket = quantize_heap_size(size);
            assert!(bucket >= previous);
            assert_eq!(bucket % 100_000, 0);
            previous = bucket;
        }
        assert_eq!(quantize_heap_size(usize::MAX), previous);
        assert!((3_000_000_000..4_000_000_000).contains(&previous));
    }
}
