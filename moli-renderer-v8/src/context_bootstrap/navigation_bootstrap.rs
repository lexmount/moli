use super::location_history_storage::WINDOW_RUNTIME_OWNER_SLOT;
use super::location_runtime::{
    build_location_runtime_object, install_location_runtime_state,
    sync_window_location_history_navigation_runtime_surface,
};
use super::navigation_activation::{
    install_navigation_activation_runtime_state, set_navigation_current_entry,
};
use super::navigation_entry::{set_history_entries, set_history_index};
use super::navigation_projection::set_history_length_from_visible_entries;
use super::navigation_result::clear_active_cross_document_navigation_if_matches;
use super::navigation_seed::{
    build_current_navigation_entry_from_seed, build_history_entries_array_from_seed,
    initial_navigation_history_seed,
};
use super::navigation_surface::{
    build_history_runtime_state, build_navigation_runtime_state,
    install_history_scroll_restoration_runtime_state, install_history_state_runtime_state,
};
use super::navigation_window::set_runtime_window_owner;
use super::*;
use crate::util::{get_private_value, set_private_value};
use anyhow::Result;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Location", require_prototype)]
struct LocationRuntimeObjectDeclaration<'scope> {
    #[webapi(slot = WINDOW_RUNTIME_OWNER_SLOT)]
    owner: v8::Local<'scope, v8::Object>,
    #[webapi(slot = WINDOW_LOCATION_HREF_SLOT)]
    href: String,
}

fn new_location_runtime_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    href: &str,
) -> Result<v8::Local<'s, v8::Object>> {
    let location = build_location_runtime_object(scope)?;
    LocationRuntimeObjectDeclaration::new(window, href.to_owned())
        .initialize(scope, location)
        .map_err(|error| anyhow::anyhow!("failed to initialize Location object: {error}"))?;
    Ok(location)
}

pub(crate) fn install_window_location_history_navigation_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    href: &str,
) -> Result<()> {
    let location = new_location_runtime_object(scope, window, href)?;
    install_location_runtime_state(scope, location, href)?;
    set_private_value(scope, window, WINDOW_LOCATION_SLOT, location.into());

    let initial_seed = initial_navigation_history_seed(scope, window, href);
    let history = build_history_runtime_state(scope, window, &initial_seed)?;
    set_runtime_window_owner(scope, history, window);
    set_private_value(scope, window, WINDOW_HISTORY_SLOT, history.into());

    let navigation = build_navigation_runtime_state(scope, window, &initial_seed)?;
    set_runtime_window_owner(scope, navigation, window);
    set_private_value(scope, window, WINDOW_NAVIGATION_SLOT, navigation.into());
    if !window.strict_equals(scope.get_current_context().global(scope).into()) {
        sync_window_location_history_navigation_runtime_surface(scope, window);
    }
    Ok(())
}

pub(crate) fn reset_window_location_history_navigation_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    href: &str,
) -> Result<()> {
    let location = match window_runtime_object(scope, window, WINDOW_LOCATION_SLOT) {
        Some(location) => location,
        None => {
            let location = new_location_runtime_object(scope, window, href)?;
            set_private_value(scope, window, WINDOW_LOCATION_SLOT, location.into());
            location
        }
    };
    LocationRuntimeObjectDeclaration::new(window, href.to_owned())
        .initialize(scope, location)
        .map_err(|error| anyhow::anyhow!("failed to initialize Location object: {error}"))?;
    install_location_runtime_state(scope, location, href)?;

    let initial_seed = initial_navigation_history_seed(scope, window, href);
    let history = match window_runtime_object(scope, window, WINDOW_HISTORY_SLOT) {
        Some(history) => history,
        None => build_history_runtime_state(scope, window, &initial_seed)?,
    };
    set_runtime_window_owner(scope, history, window);
    install_history_scroll_restoration_runtime_state(scope, history, "auto");
    install_history_state_runtime_state(scope, history, v8::null(scope).into());
    let entries = build_history_entries_array_from_seed(scope, window, &initial_seed);
    let current_entry = entries
        .get_index(scope, initial_seed.current_index)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or_else(|| {
            build_current_navigation_entry_from_seed(
                scope,
                window,
                &initial_seed,
                v8::null(scope).into(),
            )
        });
    set_history_entries(scope, history, entries);
    set_history_index(scope, history, initial_seed.current_index);
    set_history_length_from_visible_entries(scope, history, entries);
    set_private_value(scope, window, WINDOW_HISTORY_SLOT, history.into());

    let navigation = match window_runtime_object(scope, window, WINDOW_NAVIGATION_SLOT) {
        Some(navigation) => navigation,
        None => build_navigation_runtime_state(scope, window, &initial_seed)?,
    };
    set_runtime_window_owner(scope, navigation, window);
    set_navigation_current_entry(scope, navigation, current_entry);
    clear_active_cross_document_navigation_if_matches(scope, navigation, href);
    install_navigation_activation_runtime_state(
        scope,
        navigation,
        current_entry,
        initial_seed.activation.as_ref(),
    );
    set_private_value(scope, window, WINDOW_NAVIGATION_SLOT, navigation.into());

    sync_window_location_history_navigation_runtime_surface(scope, window);
    Ok(())
}

fn window_runtime_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, window, slot)
        .filter(|value| !value.is_undefined())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}
