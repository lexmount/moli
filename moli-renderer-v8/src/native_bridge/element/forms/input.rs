mod association;
mod checked;
mod defaults;
mod files;
mod numeric;
mod reflected;
mod textarea;
mod value;

pub(in crate::native_bridge) use self::association::input_list_getter_function;
pub(in crate::native_bridge) use self::checked::{
    input_checked_getter_function, input_checked_setter_function,
    input_indeterminate_getter_function, input_indeterminate_setter_function,
};
pub(in crate::native_bridge) use self::defaults::{
    input_default_checked_getter_function, input_default_checked_setter_function,
    input_default_value_getter_function, input_default_value_setter_function,
};
pub(crate) use self::files::{cache_input_files_from_selected_files, input_files_for_object};
pub(in crate::native_bridge) use self::files::{
    input_files_getter_function, input_files_setter_function,
};
pub(in crate::native_bridge) use self::numeric::{
    input_max_length_getter_function, input_max_length_setter_function,
    input_min_length_getter_function, input_min_length_setter_function, input_size_getter_function,
    input_size_setter_function, textarea_max_length_getter_function,
    textarea_max_length_setter_function, textarea_min_length_getter_function,
    textarea_min_length_setter_function,
};
pub(in crate::native_bridge) use self::reflected::{
    input_accept_getter_function, input_accept_setter_function, input_alt_getter_function,
    input_alt_setter_function, input_dir_name_getter_function, input_dir_name_setter_function,
    input_disabled_getter_function, input_disabled_setter_function,
    input_form_action_getter_function, input_form_action_setter_function,
    input_form_enctype_getter_function, input_form_enctype_setter_function,
    input_form_method_getter_function, input_form_method_setter_function,
    input_form_no_validate_getter_function, input_form_no_validate_setter_function,
    input_form_target_getter_function, input_form_target_setter_function,
    input_height_getter_function, input_height_setter_function, input_max_getter_function,
    input_max_setter_function, input_min_getter_function, input_min_setter_function,
    input_multiple_getter_function, input_multiple_setter_function, input_pattern_getter_function,
    input_pattern_setter_function, input_placeholder_getter_function,
    input_placeholder_setter_function, input_read_only_getter_function,
    input_read_only_setter_function, input_required_getter_function,
    input_required_setter_function, input_src_getter_function, input_src_setter_function,
    input_step_getter_function, input_step_setter_function, input_width_getter_function,
    input_width_setter_function,
};
pub(in crate::native_bridge) use self::textarea::{
    textarea_cols_getter_function, textarea_cols_setter_function,
    textarea_default_value_getter_function, textarea_default_value_setter_function,
    textarea_dir_name_getter_function, textarea_dir_name_setter_function,
    textarea_disabled_getter_function, textarea_disabled_setter_function,
    textarea_placeholder_getter_function, textarea_placeholder_setter_function,
    textarea_read_only_getter_function, textarea_read_only_setter_function,
    textarea_required_getter_function, textarea_required_setter_function,
    textarea_rows_getter_function, textarea_rows_setter_function,
    textarea_text_length_getter_function, textarea_type_getter_function,
    textarea_wrap_getter_function, textarea_wrap_setter_function,
};
pub(in crate::native_bridge) use self::value::{
    input_step_down_callback, input_step_up_callback, input_type_getter_function,
    input_type_setter_function, input_value_as_date_getter_function,
    input_value_as_date_setter_function, input_value_as_number_getter_function,
    input_value_as_number_setter_function, input_value_getter_function,
    input_value_setter_function,
};
