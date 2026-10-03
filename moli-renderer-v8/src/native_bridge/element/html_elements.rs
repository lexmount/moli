use crate::document_runtime::DomHandle;
use crate::dom::forms::parse_html_integer_prefix;

use super::{PageScriptHost, reflected_attribute};

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
    runtime: &PageScriptHost,
    handle: DomHandle,
    name: &str,
    default: i32,
) -> i32 {
    reflected_attribute(runtime, handle, name)
        .and_then(|value| parse_html_integer_prefix(&value))
        .unwrap_or(default)
}
