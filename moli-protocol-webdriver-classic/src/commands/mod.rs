mod alerts;
mod cookies;
mod elements;
mod errors;
mod navigation;
mod parsing;
mod script;
mod window;

pub(crate) use parsing::{required_object_string, required_timeout_value};

pub use alerts::{alert_handle_command, alert_send_text_command, alert_text_command};
pub use cookies::{
    add_cookie_command, classic_cookie_by_name, classic_cookies_from_devtools,
    delete_all_cookies_command, delete_cookie_command, get_cookies_command,
};
pub use elements::{
    active_element_command, cdp_node_id_from_classic_element_id,
    cdp_node_id_from_classic_shadow_root_id, classic_attribute_value, classic_element_id,
    classic_element_reference, classic_elements_from_locate_nodes_result,
    classic_elements_from_query_result, classic_property_value, classic_rect_from_geometry,
    classic_shadow_root_id, classic_shadow_root_reference, classic_text_value,
    clear_element_command, describe_node_command, describe_node_reference_command,
    element_click_command, element_click_prepare_commands,
    element_click_prepare_reference_commands, find_element_command, find_element_command_with_root,
    frame_id_for_element_command, get_element_attributes_command,
    get_element_attributes_reference_command, get_element_computed_label_command,
    get_element_computed_role_command, get_element_css_value_command,
    get_element_displayed_command, get_element_enabled_command, get_element_property_command,
    get_element_property_reference_command, get_element_rect_command,
    get_element_rect_reference_command, get_element_rendered_text_command,
    get_element_shadow_root_command, get_element_tag_name_command, get_element_text_command,
    get_element_text_reference_command, release_remote_object_command, resolve_element_command,
    resolve_element_reference_command, resolve_element_reference_command_with_execution_context,
    resolve_shadow_root_command, resolve_shadow_root_reference_command,
    resolve_shadow_root_reference_command_with_execution_context, shadow_root_attached_command,
    verify_element_attached_command,
};
pub use errors::classic_error_from_devtools_error;
pub use navigation::{
    current_url_command, history_traversal_entry, navigate_command, navigation_history_command,
    page_source_command, refresh_command, title_command, traverse_history_command,
};
pub use script::{execute_async_command, execute_sync_command};
pub use window::{
    CLASSIC_HEADLESS_AVAILABLE_HEIGHT, CLASSIC_HEADLESS_SCREEN_HEIGHT,
    CLASSIC_HEADLESS_SCREEN_WIDTH, ClassicWindowPosition, ClassicWindowRect,
    ClassicWindowRectUpdate, ClassicWindowState, activate_window_command,
    classic_window_rect_for_state, classic_window_rect_from_metrics, close_window_command,
    create_initial_target_command, element_screenshot_command, layout_metrics_command,
    new_window_command, new_window_type, print_page_command, screenshot_command,
    set_window_normal_surface_state_command, set_window_rect_command,
    set_window_rect_command_with_screen, set_window_rect_update, set_window_state_command,
    set_window_surface_state_command, switch_window_command, window_handles_command,
    window_handles_from_targets,
};
