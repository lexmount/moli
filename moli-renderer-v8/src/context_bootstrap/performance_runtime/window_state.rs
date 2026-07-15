use super::{
    WINDOW_PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES,
    install::{create_performance_object, install_performance_observer_runtime_state},
};
use crate::{
    context_bootstrap::{
        WindowLazySurface, ensure_window_lazy_surface_object,
        rematerialize_window_lazy_surface_if_cached, unix_epoch_millis,
    },
    util::{get_private_value, set_private_value, v8_string},
};
use anyhow::{Result, anyhow};

mod pending;

use pending::clear_pending_window_performance_state;
pub(super) use pending::{
    increment_pending_event_count, queue_pending_resource_entry,
    record_pending_dom_content_loaded_end, record_pending_dom_content_loaded_start,
    record_pending_load_end, record_pending_load_start, take_pending_event_counts,
    take_pending_lifecycle_timestamps, take_pending_resource_entries,
};

const WINDOW_PERFORMANCE_TIME_ORIGIN_SEED_SLOT: &str = "__moliWindowPerformanceTimeOriginSeed";
const WINDOW_PERFORMANCE_NAVIGATION_TYPE_SEED_SLOT: &str =
    "__moliWindowPerformanceNavigationTypeSeed";

pub(crate) fn bind_window_performance_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    navigation_type: &str,
    time_origin: f64,
) -> Result<()> {
    let relevant_context = window
        .get_creation_context(scope)
        .ok_or_else(|| anyhow!("Performance seed target has no creation context"))?;
    if relevant_context != scope.get_current_context() {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let target_window = relevant_context.global(target_scope);
        return bind_window_performance_seed_in_current_realm(
            target_scope,
            target_window,
            navigation_type,
            time_origin,
        );
    }
    let target_window = relevant_context.global(scope);
    bind_window_performance_seed_in_current_realm(
        scope,
        target_window,
        navigation_type,
        time_origin,
    )
}

fn bind_window_performance_seed_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    navigation_type: &str,
    time_origin: f64,
) -> Result<()> {
    if time_origin_seed_value(scope, window) == Some(time_origin)
        && navigation_type_seed_value(scope, window).as_deref() == Some(navigation_type)
    {
        return Ok(());
    }
    let navigation_type = v8_string(scope, navigation_type)
        .ok_or_else(|| anyhow!("failed to allocate Performance navigation type seed"))?;
    let time_origin = v8::Number::new(scope, time_origin);
    set_private_value(
        scope,
        window,
        WINDOW_PERFORMANCE_NAVIGATION_TYPE_SEED_SLOT,
        navigation_type.into(),
    );
    set_private_value(
        scope,
        window,
        WINDOW_PERFORMANCE_TIME_ORIGIN_SEED_SLOT,
        time_origin.into(),
    );
    clear_pending_window_performance_state(scope, window);
    rematerialize_window_lazy_surface_if_cached(scope, window, WindowLazySurface::Performance)?;
    Ok(())
}

pub(in crate::context_bootstrap) fn install_default_window_performance_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Result<()> {
    install_performance_observer_runtime_state(
        scope,
        window,
        WINDOW_PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES,
    )?;
    bind_window_performance_seed(scope, window, "navigate", unix_epoch_millis())
}

pub(in crate::context_bootstrap) fn build_window_performance_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let navigation_type = navigation_type_seed(scope, window);
    let time_origin = time_origin_seed(scope, window);
    create_performance_object(scope, Some(window), &navigation_type, time_origin)
}

pub(in crate::context_bootstrap) fn finish_window_performance_materialization<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    performance: v8::Local<'s, v8::Object>,
) {
    super::install::apply_pending_window_performance_state(scope, window, performance);
}

pub(in crate::context_bootstrap) fn ensure_current_window_performance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    ensure_window_lazy_surface_object(scope, global, WindowLazySurface::Performance)
}

pub(super) fn current_window_performance_time_origin_seed(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<f64> {
    let global = scope.get_current_context().global(scope);
    time_origin_seed_value(scope, global)
}

fn navigation_type_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> String {
    get_private_value(scope, window, WINDOW_PERFORMANCE_NAVIGATION_TYPE_SEED_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .map(|value| match value.as_str() {
            "reload" => "reload".to_owned(),
            "traverse" => "traverse".to_owned(),
            _ => "navigate".to_owned(),
        })
        .unwrap_or_else(|| "navigate".to_owned())
}

fn navigation_type_seed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, window, WINDOW_PERFORMANCE_NAVIGATION_TYPE_SEED_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn time_origin_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> f64 {
    get_private_value(scope, window, WINDOW_PERFORMANCE_TIME_ORIGIN_SEED_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or_else(unix_epoch_millis)
}

fn time_origin_seed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    get_private_value(scope, window, WINDOW_PERFORMANCE_TIME_ORIGIN_SEED_SLOT)
        .and_then(|value| value.number_value(scope))
}
