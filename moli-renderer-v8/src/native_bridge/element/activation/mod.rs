mod click;
mod clipboard;
mod clipboard_copy;
pub(crate) use clipboard_copy::{document_copy_command_supported, run_document_copy_command};
mod default_action;
mod targets;

pub(crate) use clipboard::perform_clipboard_key_default_action;

pub(in crate::native_bridge) use click::{
    input_show_picker_callback, node_click_callback, select_show_picker_callback,
};
pub(crate) use default_action::{
    activate_default_submit_button_via_keyboard, activate_handle_after_pointer_release,
    activate_handle_via_click, activate_handle_via_click_with_detail_and_modifiers,
    activate_handle_via_synthetic_click, dispatched_click_activation_target,
    finish_legacy_activation_for_dispatched_click, perform_auxiliary_link_default_action,
    perform_click_default_action_for_dispatched_event, perform_drop_default_action,
    prepare_legacy_activation_for_dispatched_click, replace_contenteditable_selection,
    scroll_to_document_fragment_target, scroll_to_url_fragment_or_top,
    select_contenteditable_contents,
};
pub(in crate::native_bridge) use targets::choose_form_navigation_target;
pub(crate) use targets::{
    NamedHyperlinkPopup, SpecialBrowsingContextTarget, navigate_existing_browsing_context_target,
    navigate_iframe_target,
};
