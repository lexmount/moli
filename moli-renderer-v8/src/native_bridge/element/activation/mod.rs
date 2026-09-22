mod click;
mod default_action;
mod targets;

pub(in crate::native_bridge) use click::{input_show_picker_callback, node_click_callback};
pub(in crate::native_bridge) use default_action::navigate_form_target_browsing_context;
pub(crate) use default_action::{
    activate_handle_via_click, activate_handle_via_click_with_detail_and_modifiers,
    activate_handle_via_synthetic_click, dispatched_click_activation_target,
    finish_legacy_activation_for_dispatched_click, perform_auxiliary_link_default_action,
    perform_click_default_action_for_dispatched_event, perform_drop_default_action,
    prepare_legacy_activation_for_dispatched_click, replace_contenteditable_selection,
    scroll_to_url_fragment_or_top, select_contenteditable_contents,
};
pub(in crate::native_bridge) use targets::named_iframe_target_handle_for_navigation;
pub(crate) use targets::{
    SpecialBrowsingContextTarget, navigate_existing_browsing_context_target,
    navigate_named_iframe_target,
};
pub(in crate::native_bridge::element) use targets::{
    queue_deferred_named_iframe_target_navigation_from_document,
    queue_deferred_named_iframe_target_request,
};
