mod attach;
mod internals;
mod root_accessors;
mod slots;
mod template;

pub(in crate::native_bridge) use attach::{
    element_attach_shadow_callback, shadow_root_init_from_attach_shadow_value,
};
pub(in crate::native_bridge) use internals::element_attach_internals_callback;
pub(crate) use internals::install_element_internals_template_bindings;
pub(crate) use internals::{
    element_internals_form_value_for_target,
    element_internals_validation_message_for_target_handle,
    element_internals_validity_for_target_handle, element_internals_will_validate_for_handle,
};
pub(in crate::native_bridge) use root_accessors::element_shadow_root_getter_function;
pub(crate) use root_accessors::{
    clear_shadow_root_adopted_style_sheets, css_module_sheet_for_url,
    ensure_shadow_root_adopted_style_sheets_initialized,
    initialize_declarative_adopted_style_sheets, mark_css_module_sheet_fetch_failed,
};
pub(super) use root_accessors::{
    shadow_root_active_element_getter_function, shadow_root_adopted_style_sheets_getter_function,
    shadow_root_adopted_style_sheets_setter_function, shadow_root_clonable_getter_function,
    shadow_root_delegates_focus_getter_function, shadow_root_host_getter_function,
    shadow_root_mode_getter_function, shadow_root_reference_target_getter_function,
    shadow_root_reference_target_setter_function, shadow_root_serializable_getter_function,
    shadow_root_slot_assignment_getter_function, shadow_root_style_sheets_getter_function,
};
pub(in crate::native_bridge) use slots::{node_slot_getter_function, node_slot_setter_function};
pub(super) use slots::{
    slot_assign_callback, slot_assigned_elements_callback, slot_assigned_nodes_callback,
    slot_assigned_slot_getter_function, slot_name_getter_function, slot_name_setter_function,
};
pub(super) use template::{
    template_content_getter_function, template_shadow_root_adopted_style_sheets_getter_function,
    template_shadow_root_adopted_style_sheets_setter_function,
    template_shadow_root_clonable_getter_function, template_shadow_root_clonable_setter_function,
    template_shadow_root_custom_element_registry_getter_function,
    template_shadow_root_custom_element_registry_setter_function,
    template_shadow_root_delegates_focus_getter_function,
    template_shadow_root_delegates_focus_setter_function,
    template_shadow_root_mode_getter_function, template_shadow_root_mode_setter_function,
    template_shadow_root_serializable_getter_function,
    template_shadow_root_serializable_setter_function,
    template_shadow_root_slot_assignment_getter_function,
    template_shadow_root_slot_assignment_setter_function,
};
