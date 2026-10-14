use super::*;

mod accessors;
mod builder;
mod install;
mod value;

pub(crate) use builder::new_attr_object;
pub(in crate::native_bridge::document) use value::attr_current_string_value;
