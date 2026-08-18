use super::history_mutation::{history_push_state_callback, history_replace_state_callback};
use super::location_history_storage::{
    HISTORY_ENTRIES_SLOT, HISTORY_INDEX_SLOT, HISTORY_LENGTH_SLOT, HISTORY_SCROLL_RESTORATION_SLOT,
    HISTORY_STATE_SLOT, NAVIGATION_CURRENT_ENTRY_SLOT, NAVIGATION_EVENT_LISTENERS_SLOT,
};
use super::navigation_activation::install_navigation_activation_runtime_state;
use super::navigation_callbacks::{
    navigation_entries_callback, navigation_navigate_callback,
    navigation_update_current_entry_callback,
};
use super::navigation_events::navigation_error_event_active;
use super::navigation_projection::set_history_length_from_visible_entries;
use super::navigation_seed::{
    build_current_navigation_entry_from_seed, build_history_entries_array_from_seed,
};
use super::navigation_traversal::{
    history_back_callback, history_forward_callback, history_go_callback, navigation_back_callback,
    navigation_entries_len, navigation_forward_callback, navigation_reload_callback,
    navigation_traverse_to_callback, pending_or_current_navigation_entry_index,
};
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, navigation_document_has_opaque_origin,
    navigation_document_is_active, runtime_window_owner, window_history_for_holder,
};
use super::*;
use anyhow::Result;

mod accessors;
mod bindings;
mod runtime;

pub(super) use bindings::{install_history_bindings, install_navigation_bindings};
pub(super) use runtime::{
    build_history_runtime_state, build_navigation_runtime_state,
    install_history_scroll_restoration_runtime_state, install_history_state_runtime_state,
};
