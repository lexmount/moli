mod attrs;
mod converter;
mod expand;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Derives positional WebIDL argument parsing for V8 native binding structs.
///
/// The generated implementation reads fields from `v8::FunctionCallbackArguments`
/// and returns a Rust struct. Field attributes control required arguments,
/// explicit indexes, defaults, nullable values, variadic tails, custom
/// converters, and hand-written parser hooks.
/// `#[webidl(interface = Type)]` validates a V8 object using `Type::NAME` and
/// `Type::is_instance`, with an optional `brand_check = path` override. It
/// composes with optional/nullable fields and variadic object arguments.
/// Non-optional interface fields without a default convert at their field
/// position; `required` additionally enables the initial arity check.
/// `#[webidl(dictionary)]` delegates to a dictionary derive. Missing/nullish
/// values parse an empty dictionary; optional fields skip missing/undefined.
/// `#[webidl(sequence)]` converts one iterable to `Vec<T>`, using inferred or
/// explicit item conversion, including `interface` and `brand_check` metadata.
#[proc_macro_derive(WebIdlArgs, attributes(webidl))]
pub fn derive_webidl_args(input: TokenStream) -> TokenStream {
    match expand::expand_webidl_args(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Derives named WebIDL dictionary member parsing for Rust structs.
///
/// The generated implementation reads object properties with ordinary V8
/// property access so getter side effects and getter exceptions are preserved.
/// Field attributes control member names, required/default handling,
/// `nullable`, legacy nullish handling, explicit converters, and hand-written
/// member parser hooks. Unnamed fields use `camelCase` member names by default,
/// matching common WebIDL dictionary spelling.
/// Members are read and converted in lexicographical order of their final
/// JavaScript names, including `name` and `rename_all` overrides. Rust raw
/// identifiers such as `r#type` refer to the member `type`.
/// One `#[webidl(inherit)]` field delegates to its `WebIdlDictionary` type on
/// the same object before all own members, without reading a property for the
/// field. Inheritance cannot be combined with member/conversion attributes.
/// Object members support `interface = Type` and an optional `brand_check = path`
/// using the same native identity conversion as positional arguments.
/// Nested members support `dictionary`: null parses an empty dictionary while
/// `Option<T>` skips missing/undefined and `nullable` also skips null.
/// Iterable members support `sequence` with `Vec<T>` or `Option<Vec<T>>`.
#[proc_macro_derive(WebIdlDictionary, attributes(webidl))]
pub fn derive_webidl_dictionary(input: TokenStream) -> TokenStream {
    match expand::expand_webidl_dictionary(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Derives WebIDL enum token parsing.
///
/// Unit enum variants are converted to lowercase tokens by default. Container
/// `#[webidl(rename_all = "...")]` supports `lowercase`, `kebab-case`,
/// `camelCase`, and `none`, while variant-level `#[webidl(token = "...")]`
/// overrides the generated token. Types with custom parsing can use
/// `#[webidl(parse_with = path)]`, where the path returns `Option<Self>`.
#[proc_macro_derive(WebIdlEnum, attributes(webidl))]
pub fn derive_webidl_enum(input: TokenStream) -> TokenStream {
    match expand::expand_webidl_enum(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}
