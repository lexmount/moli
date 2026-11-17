mod click;
mod clipboard;
mod clipboard_copy;
pub(crate) use clipboard_copy::{document_copy_command_supported, run_document_copy_command};
mod default_action;
mod targets;
#[cfg(feature = "devtools")]
mod text_input;

#[cfg(feature = "devtools")]
pub(crate) use text_input::TextInputSource;
#[cfg(feature = "devtools")]
pub(crate) use text_input::perform_text_input;

#[cfg(feature = "devtools")]
pub(crate) use clipboard::perform_clipboard_key_default_action;

pub(in crate::native_bridge) use click::{
    input_show_picker_callback, node_click_callback, select_show_picker_callback,
};
#[cfg(feature = "ace-protocol")]
pub(crate) use default_action::activate_handle_via_click_with_detail_and_modifiers;
pub(super) use default_action::dispatch_click_event;
#[cfg(feature = "devtools")]
pub(crate) use default_action::{
    accepts_drop_default_action, activate_handle_after_pointer_release,
    perform_auxiliary_link_default_action, perform_drop_default_action,
};
#[cfg(feature = "devtools")]
pub(crate) use default_action::{
    activate_handle_after_touch_release, activate_handle_via_keyboard,
    select_contenteditable_contents,
};
pub(crate) use default_action::{
    activate_handle_via_synthetic_click, dispatched_click_activation_target,
    finish_legacy_activation_for_dispatched_click,
    perform_click_default_action_for_dispatched_event,
    prepare_legacy_activation_for_dispatched_click, replace_contenteditable_selection,
    scroll_to_document_fragment_target, scroll_to_url_fragment_or_top,
};
pub(in crate::native_bridge) use targets::{
    FormNavigationTarget, choose_form_navigation_target, form_blocked_ancestor_targets,
    form_navigation_suppresses_referrer, form_navigation_target_name,
};
pub(crate) use targets::{
    NamedHyperlinkPopup, SpecialBrowsingContextTarget, navigate_existing_browsing_context_target,
    navigate_iframe_target,
};

#[cfg(test)]
pub(crate) use default_action::activate_handle_via_click;
