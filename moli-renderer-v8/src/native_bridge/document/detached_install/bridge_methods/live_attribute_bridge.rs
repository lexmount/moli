use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record)]
struct LiveAttributeBridgeMethodsDeclaration {
    #[webapi(
        method = "__setAttributeNodeForLiveElement",
        callback = bridge_set_attribute_node_for_live_element_callback
    )]
    set_attribute_node_for_live_element: (),

    #[webapi(
        method = "__removeAttributeNodeForLiveElement",
        callback = bridge_remove_attribute_node_for_live_element_callback
    )]
    remove_attribute_node_for_live_element: (),
}

pub(super) fn install_live_attribute_bridge_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bridge: v8::Local<'s, v8::Object>,
) {
    LiveAttributeBridgeMethodsDeclaration::new()
        .initialize(scope, bridge)
        .expect("live attribute bridge method declaration should initialize");
}
