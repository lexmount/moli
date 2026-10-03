use anyhow::{Result, anyhow};

use crate::util::{get_private_value, set_private_value, v8str};

const CACHE: &str = "__moliCrossOriginWindowSurfaceCache";

/// Capture the builtins before author code runs. The array is owned by V8's
/// realm graph, so neither the cache nor its functions root a retired realm.
/// Slot zero starts as the constructor and becomes a WeakMap on first use.
pub(in crate::context_bootstrap) fn initialize_cross_origin_window_cache(
    scope: &mut v8::PinScope<'_, '_>,
) -> Result<()> {
    let context = scope.get_current_context();
    let anchor = context.get_extras_binding_object(scope);
    if get_private_value(scope, anchor, CACHE).is_some() {
        return Ok(());
    }
    let constructor = context
        .global(scope)
        .get(scope, v8str(scope, "WeakMap").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing intrinsic WeakMap constructor"))?;
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing intrinsic WeakMap prototype"))?;
    let [get, set] = ["get", "set"].map(|name| {
        prototype
            .get(scope, v8str(scope, name).into())
            .filter(|value| value.is_function())
            .ok_or_else(|| anyhow!("missing intrinsic WeakMap.{name}"))
    });
    let entries = [constructor.into(), get?, set?];
    let cache = v8::Array::new_with_elements(scope, &entries);
    set_private_value(scope, anchor, CACHE, cache.into());
    Ok(())
}

/// Each target owns an ephemeron cache keyed by the accessing realm. This
/// preserves per-observer function identity without retaining dead observers.
pub(crate) fn cached_cross_origin_window_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Context>,
    accessing: v8::Local<'s, v8::Context>,
    build: impl FnOnce(&mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let (map, get, set) = {
        let scope = &mut v8::ContextScope::new(scope, target);
        let anchor = target.get_extras_binding_object(scope);
        let cache = get_private_value(scope, anchor, CACHE)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .expect("Window cross-origin cache initialized during bootstrap");
        let value = cache.get_index(scope, 0).expect("Window cache slot");
        let map = if let Ok(constructor) = v8::Local::<v8::Function>::try_from(value) {
            let map = constructor
                .new_instance(scope, &[])
                .expect("intrinsic WeakMap construction");
            assert_eq!(cache.set_index(scope, 0, map.into()), Some(true));
            map
        } else {
            v8::Local::<v8::Object>::try_from(value).expect("Window surface WeakMap")
        };
        let method = |scope: &mut v8::PinScope<'s, '_>, index| {
            cache
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .expect("captured intrinsic WeakMap method")
        };
        (map, method(scope, 1), method(scope, 2))
    };
    let scope = &mut v8::ContextScope::new(scope, accessing);
    let key = accessing.get_extras_binding_object(scope);
    let existing = get
        .call(scope, map.into(), &[key.into()])
        .expect("intrinsic WeakMap.get");
    if let Ok(surface) = v8::Local::<v8::Object>::try_from(existing) {
        return surface;
    }
    let surface = build(scope);
    set.call(scope, map.into(), &[key.into(), surface.into()])
        .expect("intrinsic WeakMap.set");
    surface
}
