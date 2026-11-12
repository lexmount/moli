use super::super::{
    BridgeHandle, DomTokenListKind, PageScriptHost, bridge_handle_from_object,
    node::{node_is_element, node_runtime_and_handle_from_object_or_detached},
    throw_dom_exception, validate_class_list_token, validate_class_list_token_pair,
};
use super::{element_attribute, property_dom_string_value, set_reflected_attribute};
use crate::{document_runtime::DomHandle, util::v8_string};

mod accessors;
mod identity;
mod indexed;
mod methods;
mod properties;
mod template;
mod tokens;

pub(in crate::native_bridge) use self::accessors::{
    html_rel_list_getter_function, html_rel_list_setter_function,
};
pub(in crate::native_bridge) use self::template::build_dom_token_list_wrapper_template;
pub(crate) use self::template::install_dom_token_list_prototype_bindings;
