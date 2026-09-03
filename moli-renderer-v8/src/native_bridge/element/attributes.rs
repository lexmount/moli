mod attr_nodes;
mod bridge_callbacks;
mod getters;
mod inspector;
mod mutation;

#[derive(crate::webidl::WebIdlArgs)]
#[webidl(prefix = "Element attribute")]
struct AttributeNameArgs {
    #[webidl(required)]
    name: String,
}

#[derive(crate::webidl::WebIdlArgs)]
#[webidl(prefix = "Element attributeNS")]
struct AttributeNamespaceNameArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    local_name: String,
}

#[derive(crate::webidl::WebIdlArgs)]
#[webidl(prefix = "Element setAttribute")]
struct SetAttributeArgs<'s> {
    #[webidl(required)]
    name: String,
    #[webidl(required, converter = "raw")]
    value: v8::Local<'s, v8::Value>,
}

#[derive(crate::webidl::WebIdlArgs)]
#[webidl(prefix = "Element setAttributeNS")]
struct SetAttributeNsArgs<'s> {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    qualified_name: String,
    #[webidl(required, converter = "raw")]
    value: v8::Local<'s, v8::Value>,
}

#[derive(crate::webidl::WebIdlArgs)]
#[webidl(prefix = "Element toggleAttribute")]
struct ToggleAttributeArgs {
    #[webidl(required)]
    name: String,
    force: Option<bool>,
}

pub(in crate::native_bridge) use self::attr_nodes::{
    node_get_attribute_node_callback, node_get_attribute_node_ns_callback,
    node_remove_attribute_node_callback, node_set_attribute_node_callback,
};
pub(in crate::native_bridge) use self::bridge_callbacks::{
    bridge_get_attribute_callback, bridge_remove_attribute_callback, bridge_set_attribute_callback,
};
pub(in crate::native_bridge) use self::getters::{
    node_get_attribute_callback, node_get_attribute_names_callback, node_get_attribute_ns_callback,
    node_has_attribute_callback, node_has_attribute_ns_callback, node_has_attributes_callback,
};
pub(crate) use self::inspector::mutate_live_element_attribute_for_inspector;
pub(in crate::native_bridge) use self::mutation::{
    node_remove_attribute_callback, node_remove_attribute_ns_callback, node_set_attribute_callback,
    node_set_attribute_ns_callback, node_toggle_attribute_callback,
};
