use crate::document_runtime::DomHandle;

use super::super::{
    JsContextHost,
    node::{node_is_document, node_runtime_and_handle_from_object_or_detached},
    set_wrapped_handle_or_null_for_receiver, throw_dom_exception,
};

mod focus;
mod metadata;

pub(in crate::native_bridge) use focus::node_document_active_element_getter_function;
pub(in crate::native_bridge) use metadata::throw_document_domain_security_error;
