//! ValidityState values shared by native form controls and ElementInternals.

use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ValidityState, data_properties, enumerable)]
pub struct ValidityStateDeclaration {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub pattern_mismatch: bool,
    pub too_long: bool,
    pub too_short: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
    pub custom_error: bool,
    pub valid: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ValidityState, data_properties, enumerable)]
pub struct ValidityStateFlagsUpdateDeclaration {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub pattern_mismatch: bool,
    pub too_long: bool,
    pub too_short: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
    pub custom_error: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ValidityState, data_properties, enumerable)]
pub struct ValidityStateValidUpdateDeclaration {
    pub valid: bool,
}
