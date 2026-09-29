use super::*;

pub(in crate::native_bridge) use attr_object::AttrReference;
pub(crate) use attr_object::install_attr_prototype;

mod attr_object;
mod attribute_node;
mod named_node_map;

pub(crate) use attr_object::live_get_attribute_node_object;
pub(crate) use attr_object::new_attr_object;
pub(super) use attr_object::{attr_state_object, live_attr_cache_object};
pub(in crate::native_bridge) use attr_object::{
    clear_live_attr_cache_entry, clear_live_attr_cache_entry_ns,
};
pub(in crate::native_bridge) use attr_object::{
    is_attr_node_value, live_get_attribute_node_ns_object,
};
pub(in crate::native_bridge::document) use attr_object::{
    namespace_attr_cache_key, set_attr_cache_entry,
};
pub(in crate::native_bridge::document) use attribute_node::native_attr_object_from_snapshot;
pub(super) use attribute_node::{
    bridge_remove_attribute_node_for_live_element_callback,
    bridge_set_attribute_node_for_live_element_callback, detached_create_attribute_method_callback,
    detached_create_attribute_ns_method_callback, detached_native_remove_attribute_node,
    detached_native_set_attribute_node,
};
pub(in crate::native_bridge) use attribute_node::{
    detached_get_attribute_node_method_callback, detached_get_attribute_node_ns_method_callback,
    detached_remove_attribute_node_method_callback, detached_set_attribute_node_method_callback,
};
pub(in crate::native_bridge) use named_node_map::build_named_node_map_wrapper_template;
pub(crate) use named_node_map::install_named_node_map_template_bindings;
pub(crate) use named_node_map::live_named_node_map_wrapper;
