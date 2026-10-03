use super::helpers::{attribute_node_for_index, named_node_map_attribute_names};
use super::*;
use crate::util::{get_private_value, set_private_value};

const NAMED_NODE_MAP_CACHE_SLOT: &str = "__moliNamedNodeMapCache";
pub(super) const NAMED_NODE_MAP_ELEMENT_SLOT: &str = "__moliNamedNodeMapElement";

fn live_named_node_map_cache<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, element, NAMED_NODE_MAP_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_live_named_node_map_cache<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, element, NAMED_NODE_MAP_CACHE_SLOT, wrapper.into());
}

pub(crate) fn live_named_node_map_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    element: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(wrapper) = live_named_node_map_cache(scope, element) {
        refresh_named_node_map_wrapper(scope, wrapper, element);
        return wrapper;
    }
    // The attributes getter already resolved this host from the receiver.
    // The public bridge can be absent while that host is still installed.
    let template = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .named_node_map_wrapper_template();
    let wrapper = template
        .new_instance(scope)
        .expect("failed to instantiate NamedNodeMap wrapper");
    assert!(
        wrapper.set_internal_field(0, element.into()),
        "NamedNodeMap wrapper must expose its owner element field"
    );
    super::super::super::super::bindings::set_named_constructor_prototype(
        scope,
        wrapper,
        "NamedNodeMap",
    );
    refresh_named_node_map_wrapper(scope, wrapper, element);
    set_live_named_node_map_cache(scope, element, wrapper);
    wrapper
}

fn refresh_named_node_map_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, wrapper, NAMED_NODE_MAP_ELEMENT_SLOT, element.into());
    let names = named_node_map_attribute_names(scope, element);
    for index in 0..names.len() {
        let Some(attr) = attribute_node_for_index(scope, element, index) else {
            continue;
        };
        let _ = wrapper.set_index(scope, index as u32, attr);
    }
}
