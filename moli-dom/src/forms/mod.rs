mod button_type;
mod input_type;
mod numeric;
mod option;
mod text;
mod validity;

pub use button_type::ButtonTypeState;
pub(crate) use input_type::{
    InputValueSanitizationContext, sanitize_input_value_for_type_with_context,
};
pub use input_type::{
    email_value_type_mismatch, form_control_type_supports_intrinsic_validation,
    input_type_has_value_sanitization, input_type_value_mismatch, is_valid_email_address,
    sanitize_input_value_for_type, sanitize_input_value_for_type_with_multiple,
    url_value_type_mismatch,
};
pub use moli_html_input_temporal::{
    MS_PER_DAY, MS_PER_HOUR, MS_PER_MINUTE, MS_PER_SECOND, MS_PER_WEEK, WEEK_INPUT_STEP_BASE,
    date_input_milliseconds, date_input_value_from_milliseconds, datetime_local_input_milliseconds,
    datetime_local_input_value_from_milliseconds, is_valid_date_input_value,
    is_valid_datetime_local_input_value, is_valid_month_input_value, is_valid_time_input_value,
    is_valid_week_input_value, month_input_milliseconds, month_input_number,
    month_input_value_from_milliseconds, month_input_value_from_number, time_input_milliseconds,
    time_input_value_from_milliseconds, week_input_milliseconds,
    week_input_value_from_milliseconds,
};
pub use moli_html_input_type::InputType;
pub use numeric::{
    InputStepDirection, InputStepError, InputStepOutcome, InputStepState, MeterElementValues,
    MeterGaugeRegion, ProgressElementValues, input_number_to_value_string, input_range_overflow,
    input_range_underflow, input_step, input_step_base, is_valid_number_input_value,
    meter_element_values, number_aligns_to_step, number_step_mismatch,
    parse_html_floating_point_prefix, parse_input_numeric_value, progress_element_values,
    step_input_value,
};
pub use option::{
    OptionDisabledAncestorStep, OptionNearestSelectStep, OptionNearestSelectTraversal,
    option_disabled_ancestor_step,
};
pub use text::{
    apply_textarea_wrapping_transformation, normalize_custom_validation_message,
    normalize_form_submission_newlines, parse_non_negative_integer_prefix,
    parse_non_negative_length_attribute, parse_positive_integer_prefix,
    text_control_suffers_too_long, text_control_suffers_too_short, text_control_value_length,
};
pub use validity::FormControlValidity;

#[cfg(test)]
mod tests;
