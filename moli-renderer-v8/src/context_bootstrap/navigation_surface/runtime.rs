use super::super::navigation_entry::{
    history_entries, history_index, set_history_scroll_restoration, set_history_state,
};
use super::accessors::{
    install_history_prototype_accessors, install_navigation_prototype_accessors,
};
use super::*;
use crate::native_bridge::NavigationHistoryEntrySeed;
use crate::util::get_private_value;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "History")]
struct HistoryRuntimeObjectDeclaration<'scope> {
    #[webapi(slot = HISTORY_STATE_SLOT)]
    state: v8::Local<'scope, v8::Value>,

    #[webapi(slot = HISTORY_SCROLL_RESTORATION_SLOT)]
    scroll_restoration: &'static str,

    #[webapi(slot = HISTORY_LENGTH_SLOT)]
    length: f64,

    #[webapi(slot = HISTORY_ENTRIES_SLOT)]
    entries: v8::Local<'scope, v8::Array>,

    #[webapi(slot = HISTORY_INDEX_SLOT)]
    index: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = "Navigation")]
struct NavigationRuntimeObjectDeclaration<'scope> {
    #[webapi(slot = NAVIGATION_CURRENT_ENTRY_SLOT)]
    current_entry: v8::Local<'scope, v8::Object>,

    #[webapi(data_property = "onnavigate", init = "null")]
    onnavigate: (),

    #[webapi(data_property = "onnavigatesuccess", init = "null")]
    onnavigatesuccess: (),

    #[webapi(data_property = "onnavigateerror", init = "null")]
    onnavigateerror: (),

    #[webapi(data_property = "oncurrententrychange", init = "null")]
    oncurrententrychange: (),
}

pub(in crate::context_bootstrap) fn build_history_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    initial_seed: &NavigationHistoryEntrySeed,
) -> Result<v8::Local<'s, v8::Object>> {
    if let Some(prototype) = global_constructor_prototype(scope, "History") {
        install_history_prototype_accessors(scope, prototype);
    }
    let entries = build_history_entries_array_from_seed(scope, window, initial_seed);
    let history = HistoryRuntimeObjectDeclaration::new(
        v8::null(scope).into(),
        "auto",
        0.0,
        entries,
        initial_seed.current_index as f64,
    )
    .bind(scope)
    .map_err(anyhow::Error::from)?;
    set_history_length_from_visible_entries(scope, history, entries);
    Ok(history)
}

pub(in crate::context_bootstrap) fn build_navigation_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    initial_seed: &NavigationHistoryEntrySeed,
) -> Result<v8::Local<'s, v8::Object>> {
    if let Some(prototype) = global_constructor_prototype(scope, "Navigation") {
        install_navigation_prototype_accessors(scope, prototype);
    }
    let current_entry = window_runtime_object(scope, window, WINDOW_HISTORY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|history| {
            let entries = history_entries(scope, history)?;
            let index = history_index(scope, history);
            entries
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        })
        .unwrap_or_else(|| {
            build_current_navigation_entry_from_seed(
                scope,
                window,
                initial_seed,
                v8::null(scope).into(),
            )
        });
    let navigation = NavigationRuntimeObjectDeclaration::new(current_entry)
        .bind(scope)
        .map_err(anyhow::Error::from)?;
    super::media_queries::install_simple_event_target_methods(
        scope,
        navigation,
        NAVIGATION_EVENT_LISTENERS_SLOT,
        false,
    );
    install_navigation_activation_runtime_state(
        scope,
        navigation,
        current_entry,
        initial_seed.activation.as_ref(),
    );
    Ok(navigation)
}

fn window_runtime_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, window, slot).filter(|value| !value.is_undefined())
}

pub(in crate::context_bootstrap) fn install_history_scroll_restoration_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    value: &str,
) {
    set_history_scroll_restoration(scope, history, value);
}

pub(in crate::context_bootstrap) fn install_history_state_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    set_history_state(scope, history, state);
}
