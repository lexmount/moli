mod callbacks;
mod identity;
mod object_properties;
mod object_slots;
mod webidl;

pub(crate) use callbacks::{
    callback_arg_namespace, callback_arg_optional_string, encode_tag_name_ns_query,
};
pub(in crate::native_bridge) use identity::bridge_handle_from_object;
pub(crate) use object_properties::object_string_property;
pub(crate) use object_slots::set_object_slot;
pub(in crate::native_bridge) use webidl::webidl_long_from_number;
