mod callbacks;
mod child_script_globals;
mod identity;
mod object_properties;
mod object_slots;
mod webidl;

pub(crate) use callbacks::{
    callback_arg_namespace, callback_arg_optional_string, encode_tag_name_ns_query,
};
pub(crate) use child_script_globals::child_script_declared_global_names;
pub(in crate::native_bridge) use identity::{NATIVE_BRIDGE_OWNER_SLOT, bridge_handle_from_object};
pub(crate) use object_properties::object_string_property;
pub(crate) use object_slots::{object_has_own_named_property, set_object_slot, set_object_value};
pub(in crate::native_bridge) use webidl::webidl_long_from_number;
