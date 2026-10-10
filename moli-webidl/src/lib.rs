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
//! Dictionary members are converted in lexicographical order of their final
//! JavaScript names. `#[webidl(inherit)]` on one dictionary field converts the
//! ancestor's members from the same object first; it does not read a nested
//! property. Positional arguments retain their declaration/index order.
//!
//! Interface fields use `#[webidl(interface = interfaces::EventTarget)]` with
//! `v8::Local<'s, v8::Object>`, optionally wrapped in `Option` or a variadic
//! argument's `Vec`. The interface supplies `NAME` and `is_instance`; a
//! `brand_check = path` attribute overrides the native identity predicate.
//! `Option` skips missing/undefined values, while `nullable` additionally accepts
//! null. Identity is checked at the field's position in conversion order, and
//! the original object is retained. Interface conversion cannot be combined
//! with `converter`, `with`, or string conversion options.
//!
//! Dictionary fields use `#[webidl(dictionary)]` with a type implementing
//! `WebIdlDictionary`. Non-optional fields without an explicit default parse
//! missing/undefined/null as an empty dictionary, executing member defaults and
//! required-member checks. `Option<T>` skips missing/undefined but parses null
//! as `Some(empty_dictionary)`; `nullable` instead maps null to `None`.
//! Explicit `default = expr` replaces missing/undefined values only, following
//! the same evaluation order as other field defaults. Null still parses an
//! empty dictionary. Empty dictionaries have no inherited JavaScript properties.
//!
//! Iterable fields use `#[webidl(sequence)]` with `Vec<T>` or `Option<Vec<T>>`.
//! Item conversion is inferred from `T`, or selected by `converter = "..."`.
//! `sequence, interface = Type` validates every item using the same interface
//! metadata and optional `brand_check` as scalar interface fields. Conversion
//! reuses the runtime iterator and converts each item before reading the next.
//! `sequence` converts one iterable argument; `variadic` converts argument tails.
//!
//! Uint8Array fields retain the native view identity and reject shared,
//! resizable and growable buffers by default. `#[webidl(allow_shared)]` permits
//! fixed shared storage, as required by `[AllowShared] Uint8Array` arguments.

extern crate self as moli_webidl;

mod buffer_source;
mod convert;
mod error;
mod helpers;
mod traits;
mod types;

pub use buffer_source::{AllowSharedBufferSource, NonSharedBufferSource};
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
    Boolean, BufferSource, ByteString, ClampedUnsignedLong, ClampedUnsignedShort, Dictionary,
    DomString, DomString16, Double, EnforceRangeLong, EnforceRangeLongLong,
    EnforceRangeUnsignedLong, EnforceRangeUnsignedLongLong, EnumValue, EventListenerOptions,
    InterfaceObject, InterfaceOptions, Long, LongLong, Octet, Record, Sequence, Short,
    StringOptions, Uint8ArrayOptions, UnrestrictedDouble, UnrestrictedFloat, UnsignedLong,
    UnsignedLongLong, UnsignedShort, UsvString,
};

/// Restricted WebIDL float represented as a finite binary32 value.
pub use types::Float;
