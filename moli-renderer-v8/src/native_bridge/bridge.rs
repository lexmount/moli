use super::{BridgeIdentityStore, NativeBridgeBindings, abort, traversal};
use std::fmt;

mod callbacks;
mod dom_exception;
mod lifecycle;
mod live_collections;
mod templates;
mod traversal_state;
mod validation;
mod wrappers;

pub(crate) use callbacks::{
    callback_arg_dom_handle, callback_value_dom_handle, runtime_ptr_from_object,
    set_wrapped_handle_array, set_wrapped_handle_or_null, set_wrapped_handle_or_null_for_receiver,
    wrapped_handle_value, wrapped_handle_value_for_receiver,
};
pub(crate) use dom_exception::throw_dom_exception;
pub(crate) use lifecycle::install_detached_bridge_methods;
pub(crate) use validation::{
    validate_attribute_name, validate_class_list_token, validate_class_list_token_pair,
    validate_element_name, validate_qualified_element_name_and_namespace,
    validate_qualified_name_and_namespace,
};

pub(crate) struct NativeDomBridge {
    pub(super) bindings: NativeBridgeBindings,
    pub(super) identity: BridgeIdentityStore,
    pub(super) abort: abort::AbortStore,
    pub(super) traversal: traversal::TraversalStore,
}

impl fmt::Debug for NativeDomBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeDomBridge").finish_non_exhaustive()
    }
}
