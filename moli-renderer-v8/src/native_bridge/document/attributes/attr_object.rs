use super::*;

pub(super) const ATTR_STATE_SLOT: &str = "__moliAttrState";

mod cache;
mod instance;
mod reference;
mod state;

pub(in crate::native_bridge) use reference::AttrReference;

pub(crate) use cache::live_get_attribute_node_object;
pub(in crate::native_bridge) use cache::{
    clear_live_attr_cache_entry, clear_live_attr_cache_entry_ns, live_get_attribute_node_ns_object,
};
pub(in crate::native_bridge::document) use cache::{
    live_attr_cache_object, namespace_attr_cache_key, set_attr_cache_entry,
};
pub(crate) use instance::new_attr_object;
pub(in crate::native_bridge::document) use state::attr_state_object;

pub(in crate::native_bridge) fn is_attr_node_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    let object = v8::Global::new(scope, object);
    let object = v8::Local::new(scope, object);
    attr_state_object(scope, object).is_some()
}
