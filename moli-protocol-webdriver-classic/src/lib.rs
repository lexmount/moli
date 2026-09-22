//! WebDriver Classic protocol adapter surface.
//!
//! This crate owns WebDriver Classic wire response shapes and the HTTP session
//! registry, while browser behavior moves behind protocol-neutral DevTools
//! commands.

mod actions;
mod capabilities;
mod commands;
mod responses;
mod types;

fn geometry_border_quad(
    geometry: &moli_protocol::devtools_runtime::DevToolsDomGeometryResult,
) -> Option<&moli_protocol::devtools_runtime::DevToolsDomQuad> {
    geometry
        .box_model
        .as_ref()
        .map(|model| &model.border)
        .or_else(|| geometry.quads.first())
}

pub(crate) use commands::required_object_string;

pub use actions::{
    ClassicActionState, ClassicActionTick, ClassicElementOriginViewportPoints,
    ClassicViewportBounds, ClassicViewportPoint, action_element_origin_ids,
    element_center_from_geometry, element_click_input_commands, element_send_keys_input_commands,
    element_send_keys_prepare_text_control_command, element_send_keys_text,
    perform_actions_commands, perform_actions_commands_with_element_origins,
    perform_actions_commands_with_state, perform_actions_commands_with_state_and_viewport,
    perform_actions_ticks_with_state_and_viewport, release_actions_commands,
};
pub use capabilities::{
    CLASSIC_BROWSER_NAME, matched_capabilities_from_new_session_params,
    page_load_strategy_from_capabilities, page_load_strategy_from_new_session_params,
    unhandled_prompt_behavior_from_capabilities, unhandled_prompt_behavior_from_new_session_params,
};
pub use commands::{
    CLASSIC_HEADLESS_AVAILABLE_HEIGHT, CLASSIC_HEADLESS_SCREEN_HEIGHT,
    CLASSIC_HEADLESS_SCREEN_WIDTH, ClassicWindowPosition, ClassicWindowRect,
    ClassicWindowRectUpdate, ClassicWindowState, activate_window_command, active_element_command,
    add_cookie_command, alert_handle_command, alert_send_text_command, alert_text_command,
    cdp_node_id_from_classic_element_id, cdp_node_id_from_classic_shadow_root_id,
    classic_attribute_value, classic_cookie_by_name, classic_cookies_from_devtools,
    classic_element_id, classic_element_reference, classic_elements_from_locate_nodes_result,
    classic_elements_from_query_result, classic_error_from_devtools_error, classic_property_value,
    classic_rect_from_geometry, classic_shadow_root_id, classic_shadow_root_reference,
    classic_text_value, classic_window_rect_for_state, classic_window_rect_from_metrics,
    clear_element_command, close_window_command, create_initial_target_command,
    current_url_command, delete_all_cookies_command, delete_cookie_command, describe_node_command,
    describe_node_reference_command, element_click_command, element_click_prepare_commands,
    element_click_prepare_reference_commands, element_screenshot_command, execute_async_command,
    execute_sync_command, find_element_command, find_element_command_with_root,
    frame_id_for_element_command, get_cookies_command, get_element_attributes_command,
    get_element_attributes_reference_command, get_element_computed_label_command,
    get_element_computed_role_command, get_element_css_value_command,
    get_element_displayed_command, get_element_enabled_command, get_element_property_command,
    get_element_property_reference_command, get_element_rect_command,
    get_element_rect_reference_command, get_element_rendered_text_command,
    get_element_shadow_root_command, get_element_tag_name_command, get_element_text_command,
    get_element_text_reference_command, history_traversal_entry, layout_metrics_command,
    navigate_command, navigation_history_command, new_window_command, new_window_type,
    page_source_command, print_page_command, refresh_command, release_remote_object_command,
    resolve_element_command, resolve_element_reference_command,
    resolve_element_reference_command_with_execution_context, resolve_shadow_root_command,
    resolve_shadow_root_reference_command,
    resolve_shadow_root_reference_command_with_execution_context, screenshot_command,
    set_window_normal_surface_state_command, set_window_rect_command,
    set_window_rect_command_with_screen, set_window_rect_update, set_window_state_command,
    set_window_surface_state_command, shadow_root_attached_command, switch_window_command,
    title_command, traverse_history_command, verify_element_attached_command,
    window_handles_command, window_handles_from_targets,
};
pub use responses::{
    delete_session_response, error_response, error_response_with_data, new_session_response,
    parse_timeouts, status_response, success_response, timeouts_value,
};
pub use types::{
    CLASSIC_ELEMENT_REFERENCE_KEY, CLASSIC_FRAME_REFERENCE_KEY, CLASSIC_SHADOW_ROOT_REFERENCE_KEY,
    CLASSIC_WINDOW_REFERENCE_KEY, ClassicDevToolsCommandContext, ClassicError, ClassicErrorCode,
    ClassicPageLoadStrategy, ClassicPromptHandler, ClassicSessionRegistry, ClassicSessionState,
    ClassicTimeouts, ClassicUnhandledPromptBehavior,
};

#[cfg(test)]
mod tests;
