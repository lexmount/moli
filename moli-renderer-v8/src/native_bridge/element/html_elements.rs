use crate::document_runtime::DomHandle;

use super::{JsContextHost, reflected_attribute};

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
    reflected_attribute(runtime, handle, name)
        .and_then(|value| parse_i32_prefix(&value))
        .unwrap_or(default)
}

fn parse_i32_prefix(value: &str) -> Option<i32> {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let mut chars = value.chars();
    let (sign, rest) = match chars.next() {
        Some('+') => (1_i64, chars.as_str()),
        Some('-') => (-1_i64, chars.as_str()),
        Some(_) => (1_i64, value),
        None => return None,
    };
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let magnitude = digits.parse::<i64>().ok()?;
    i32::try_from(sign * magnitude).ok()
}
