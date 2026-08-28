use super::shared::*;
use crate::{
    custom_elements,
    native_bridge::{
        abort::dom_exception_value, node_runtime_and_handle_from_object,
        node_runtime_and_handle_from_object_or_detached,
    },
    text_codec::TextCodecStore,
    util::{
        call_global_bridge_method, context_host_ptr_from_global_bridge, global_bridge_method,
        throw_range_error, throw_type_error, v8_string, v8str,
    },
};

mod core;
mod custom_elements_registry;
mod document_nodes;
mod dom_exception;
pub(crate) use dom_exception::weaken_dom_exception_prototype_for_context_teardown;
mod dom_implementation;
mod elements;
mod text_codecs;

pub(super) use core::{
    event_target_constructor_callback, illegal_constructor_callback,
    unsupported_constructor_callback, xpath_evaluator_constructor_callback,
};
pub(super) use custom_elements_registry::{
    custom_elements_registry_constructor_callback,
    install_custom_element_registry_template_bindings,
};
pub(super) use document_nodes::{
    comment_constructor_callback, document_constructor_callback,
    document_fragment_constructor_callback, text_constructor_callback,
};
pub(crate) use dom_exception::{
    dom_error_constructor_callback, dom_exception_clone_fields, dom_exception_constructor_callback,
    finalize_dom_exception_realm_bindings, initialize_websocket_error,
    install_dom_exception_template_bindings, new_dom_error_value, new_dom_exception_value,
    new_most_derived_dom_exception_value, new_quota_exceeded_error_value,
    new_websocket_error_value, quota_exceeded_error_clone_fields,
    quota_exceeded_error_constructor_callback, throw_dom_exception_value,
    websocket_error_close_info,
};
pub(crate) use dom_implementation::ensure_dom_implementation_singleton;
pub(super) use dom_implementation::install_dom_implementation_template_bindings;
pub(super) use elements::{
    audio_constructor_callback, image_constructor_callback, option_constructor_callback,
};
pub(crate) use elements::{
    html_element_constructor_callback, html_element_constructor_with_early_sanity_trap,
};
pub(super) use text_codecs::{
    install_text_codec_template_bindings, text_decoder_constructor_callback,
    text_encoder_constructor_callback,
};
