use super::{WindowLazySurface, materialize};
use crate::util::{get_private_value, set_private_value};
use anyhow::{Result, anyhow};

pub(in crate::context_bootstrap) fn ensure_window_lazy_surface_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Result<Option<v8::Local<'s, v8::Value>>> {
    let Some(surface) = WindowLazySurface::from_slot(slot) else {
        return Ok(None);
    };
    let relevant_context = if let Some(context) = receiver.get_creation_context(scope) {
        context
    } else {
        // V8 can detach a retained WindowProxy before an associated object is
        // first read. Its own getter still enters the original native Realm.
        // Materialize there only when the receiver is that exact global.
        let context = scope.get_current_context();
        if !receiver.strict_equals(context.global(scope).into())
            || crate::util::context_host_ptr_from_context_slot(context).is_none()
        {
            return Ok(None);
        }
        context
    };
    if relevant_context == scope.get_current_context() {
        let target_window = relevant_context.global(scope);
        return ensure_in_current_realm(scope, target_window, surface).map(Some);
    }

    let value = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let target_window = relevant_context.global(target_scope);
        let value = ensure_in_current_realm(target_scope, target_window, surface)?;
        v8::Global::new(target_scope, value)
    };
    Ok(Some(v8::Local::new(scope, &value)))
}

pub(crate) fn ensure_window_lazy_surface_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) -> Result<v8::Local<'s, v8::Object>> {
    ensure_window_lazy_surface_value(scope, receiver, surface.slot())?
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("{} target has no live creation context", surface.label()))
}

/// Rebuilds an already-materialized surface from the subsystem's current seed.
///
/// Seed-only bootstrap and document rebind paths call this after updating their
/// own state. An untouched surface remains lazy; a live SameObject cache is
/// replaced together with the detached-Window callback fallback.
pub(crate) fn rematerialize_window_lazy_surface_if_cached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) -> Result<bool> {
    let relevant_context = window
        .get_creation_context(scope)
        .ok_or_else(|| anyhow!("{} rebind target has no creation context", surface.label()))?;
    if relevant_context == scope.get_current_context() {
        let target_window = relevant_context.global(scope);
        return rematerialize_in_current_realm(scope, target_window, surface);
    }
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    let target_window = relevant_context.global(target_scope);
    rematerialize_in_current_realm(target_scope, target_window, surface)
}

fn ensure_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) -> Result<v8::Local<'s, v8::Value>> {
    if let Some(value) = cached_value(scope, window, surface) {
        return Ok(value);
    }
    build_and_cache_in_current_realm(scope, window, surface)
}

fn rematerialize_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) -> Result<bool> {
    if cached_value(scope, window, surface).is_none() {
        return Ok(false);
    }
    build_and_cache_in_current_realm(scope, window, surface)?;
    Ok(true)
}

fn build_and_cache_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) -> Result<v8::Local<'s, v8::Value>> {
    if get_private_value(scope, window, surface.materializing_slot()).is_some() {
        return Err(anyhow!("reentrant {} materialization", surface.label()));
    }
    let materializing = v8::Boolean::new(scope, true);
    set_private_value(
        scope,
        window,
        surface.materializing_slot(),
        materializing.into(),
    );
    let result = materialize::build_window_lazy_surface(scope, window, surface);
    clear_private_value(scope, window, surface.materializing_slot());
    let value = result?;

    cache_value(scope, window, surface, value);
    if let Err(error) = materialize::finish_window_lazy_surface(scope, window, surface, value) {
        clear_cached_value(scope, window, surface);
        return Err(error);
    }
    Ok(value)
}

fn cached_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, window, surface.slot())
}

fn cache_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, window, surface.slot(), value);
    super::super::runtime_state::update_window_surface_detached_fallback(
        scope,
        window,
        surface.slot(),
        value,
    );
}

fn clear_cached_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) {
    let undefined = v8::undefined(scope);
    cache_value(scope, window, surface, undefined.into());
}

fn clear_private_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) {
    let undefined = v8::undefined(scope);
    set_private_value(scope, object, slot, undefined.into());
}
