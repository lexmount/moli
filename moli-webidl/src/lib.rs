//! WebIDL conversion helpers for V8 Web API binding entrypoints and Web IDL
//! callback values.
//!
//! This crate is the renderer-facing conversion layer between JavaScript values
//! and the Rust values used by Moli Web API implementations. It owns
//! argument parsing, dictionary member parsing, WebIDL scalar/string conversion,
//! and conversion error reporting. It deliberately does not construct Web API
//! objects or install interface/prototype surfaces; that belongs to
//! `moli-webapi-declare`.
//!
//! # Runtime Boundary
//!
//! Native binding entrypoints normally use `parse_args::<T>(scope, &args)` with a
//! `#[derive(WebIdlArgs)]` struct. `parse_args` converts a `WebIdlError` into a
//! thrown JavaScript `TypeError` and returns `None` so the binding can return
//! immediately. Use `try_parse_args` only when the caller needs to inspect or
//! map the error before throwing.
//!
//! Dictionary parsing follows WebIDL getter semantics: member reads go through
//! ordinary JavaScript property access, and getter exceptions are rethrown as
//! pending V8 exceptions. Optional members treat `undefined` as missing;
//! `legacy_*` helpers additionally treat `null` as missing for older browser
//! APIs that historically use nullish dictionary members.
//!
//! # Derive Boundary
//!
//! `#[derive(WebIdlArgs)]` generates positional argument parsing for a V8
//! native binding's `FunctionCallbackArguments`. `#[derive(WebIdlDictionary)]`
//! generates named property parsing for dictionary objects. Both derives use
//! the converter wrappers in `types` and only produce Rust values; object shape
//! declaration and Web API wrapper allocation remain outside this crate.

extern crate self as moli_webidl;

mod convert;
mod error;
mod helpers;
mod traits;
mod types;

pub use convert::{
    argument, argument_with_options, convert, convert_optional_sequence, convert_with_options,
    legacy_bool_member_or, legacy_number_member_or, legacy_optional_member,
    legacy_optional_member_or, legacy_optional_member_or_with_options,
    legacy_optional_member_with_options, legacy_string_member_or, non_negative_milliseconds_arg,
    number_arg_or, number_or, optional_argument_or, optional_member, optional_member_or,
    optional_member_or_with_options, optional_member_with_options, parse_args, parse_dictionary,
    parse_dictionary_object, required_argument, string_arg, timer_milliseconds_arg, try_parse_args,
};
pub use error::{Context, WebIdlError, WebIdlErrorKind};
pub use helpers::{
    add_event_listener_options_value, dictionary_arg, dictionary_value, event_listener_options,
    event_listener_options_value, is_nullish, optional_number_property, optional_object_arg,
    optional_string_property, property, property_non_nullish, property_non_undefined,
    property_result, symbol_property_result, throw_dom_exception, throw_error,
    throw_index_size_error, throw_type_error, v8_string,
};
pub use moli_webidl_callback::{
    PreparedWebIdlCallbackFunction, PreparedWebIdlCallbackInterface, WebIdlCallbackFunction,
    WebIdlCallbackInterface,
};
pub use moli_webidl_derive::{WebIdlArgs, WebIdlDictionary, WebIdlEnum};
pub use traits::{ParseOutcome, WebIdlArguments, WebIdlConverter, WebIdlDictionary, WebIdlEnum};
pub use types::{
    Boolean, BufferSource, ByteString, ClampedUnsignedShort, DomString, DomString16, Double,
    EnforceRangeLong, EnforceRangeUnsignedLong, EnforceRangeUnsignedLongLong, EnumValue,
    EventListenerOptions, Long, Record, Sequence, StringOptions, UnrestrictedDouble, UnsignedLong,
    UnsignedLongLong, UnsignedShort, UsvString,
};
