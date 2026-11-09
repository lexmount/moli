use crate::document_runtime::DomHandle;

use super::{JsContextHost, element_attribute};

mod lists;
mod marquee;
mod misc;
mod table;
mod track;

pub(super) use self::lists::*;
pub(super) use self::marquee::*;
pub(super) use self::misc::*;
pub(super) use self::table::*;
pub(super) use self::track::*;

fn parse_i32_attribute_or(
    runtime: &JsContextHost,
    handle: DomHandle,
    name: &str,
    default: i32,
) -> i32 {
    element_attribute(runtime, handle, name)
        .and_then(|value| super::global_attributes::parse_html_integer_prefix(&value))
        .unwrap_or(default)
}
