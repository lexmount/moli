//! Declarative Web API object and interface helpers.
//!
//! This crate is the Rust declaration layer that future WebIDL codegen can
//! target. It intentionally keeps conversion/parsing concerns in
//! `moli-webidl` and focuses on constructing V8 interface objects,
//! prototypes, and branded object instances.
//!
//! # Object declaration model
//!
//! A `#[derive(WebApiObject)]` struct describes the JavaScript surface that must
//! be installed on a V8 object. Field annotations decide whether a Rust field is
//! installed:
//!
//! - `#[webapi(data_property)]` defines a JavaScript own data property. It is
//!   non-enumerable by default and can opt into `enumerable`, `readonly`, or
//!   `dont_delete`. Rust slices, arrays, and vectors whose items implement
//!   `WebApiValue` are converted into V8 arrays, so fixed-shape object records
//!   can declare array-valued fields without hand-building `v8::Array`.
//! - `#[webapi(constant)]` defines a WebIDL constant own property. Constants
//!   are enumerable, read-only, and non-configurable, and must provide
//!   `value = ...`. The declaration installs the same descriptor shape used by
//!   WebIDL constants on constructors and prototypes.
//! - `#[webapi(method)]` defines a JavaScript own function property. Its
//!   `callback = ...` is the V8 callback, and optional `data = ...` is evaluated
//!   while binding the object so callbacks can receive precomputed state.
//!   Methods are non-enumerable, writable, and configurable by default; use
//!   `enumerable`, `readonly`, or `dont_delete` when the target Web API requires
//!   a different value descriptor.
//!   `symbol = "iterator"`, `symbol = "asyncIterator"`, and
//!   `symbol = "toStringTag"` use the matching well-known symbols as the
//!   property key instead of deriving a string key from the Rust field name. A
//!   method field typed as `Option<()>` is installed
//!   only when it is `Some(())`; this is intended for runtime-selected fixed own
//!   methods whose descriptors still belong to the declaration when present.
//! - `#[webapi(alias = "sourceName")]` defines an own data property whose value
//!   is copied from another already-declared own property on the same object.
//!   This is useful when Web APIs expose the same function through multiple
//!   keys, such as a string-named method and a well-known symbol, and JavaScript
//!   must observe the two properties as the exact same function object.
//! - `#[webapi(accessor_property)]` defines a JavaScript accessor property. Its
//!   `getter = ...` and optional `setter = ...` are V8 function callbacks, and
//!   optional `data = ...` follows the same callback-data model as declared
//!   methods. Accessors with different setter callback data can use
//!   `setter_data = ...`; otherwise the setter receives the same data as the
//!   getter. Use `getter_value = ...` instead of `getter = ...` only when the
//!   getter function object is already built or cached by surrounding runtime
//!   code. Accessors also support the same `symbol = ...` keys when the
//!   getter/setter pair represents a symbol-keyed Web API member.
//! - `#[webapi(native_data_property)]` defines a callback-backed V8 native data
//!   property through `Object::set_native_data_property_with_configuration`.
//!   Its `getter = ...` and optional `setter = ...` are V8 property callbacks,
//!   not JavaScript function callbacks. Reserve this for surfaces whose
//!   semantics intentionally operate on `PropertyCallbackArguments::holder()`.
//! - `#[webapi(hidden)]` defines a non-enumerable, string-named own property.
//!   This is useful for legacy bookkeeping, but it is still an own property and
//!   can be found by reflection that asks for all own property names.
//! - `#[webapi(slot)]` stores data in a V8 private slot through
//!   `moli-v8-util`. Use this for internal state that must not appear as a
//!   JavaScript own property.
//! - `#[webapi(prototype)]` and `#[webapi(to_string_tag)]` install runtime
//!   prototype and `Symbol.toStringTag` metadata from field values.
//!   `to_string_tag` fields are non-enumerable by default and can opt into
//!   `readonly` or `dont_delete` descriptor bits.
//! - `#[webapi(init = ...)]` can initialize fixed default values without a
//!   Rust value field: `null`, `undefined`, `object`, `null_object`, `array`,
//!   `true`, `false`, `0`, `""`, or `string("...")`. Object-valued defaults
//!   use named string forms such as `init = "array"`; primitive defaults use
//!   literal or typed initializer syntax.
//!
//! Field names are converted to WebIDL-style camelCase by default. Use
//! struct-level `rename_all = "none"` only for declarations that intentionally
//! expose Rust field spelling. Struct-level `#[webapi(enumerable)]` makes
//! explicitly declared string-keyed data properties, methods, accessor
//! properties, native data properties, and aliases enumerable by default. It
//! can be used with `#[webapi(data_properties)]` for plain record-like objects,
//! or by itself for WebIDL prototype declarations where every operation and
//! attribute should inherit WebIDL's enumerable prototype-member default.
//! Well-known symbol keys remain non-enumerable unless the field itself
//! declares `enumerable`.
//!
//! Fields without one of those installation annotations are declaration-only
//! inputs. The derive skips them completely, so they do not become properties or
//! private slots. They can still be referenced by method attributes such as
//! `data = self.some_field`, which lets a declaration carry callback data
//! without expanding the object reflection surface.
//!
//! Named interface instances automatically receive a private Web API type after
//! their fields initialize successfully. `bind`, `bind_into`, and `initialize`
//! all use this path. `interface = "Object"` describes an unbranded record;
//! `#[webapi(unbranded)]` explicitly opts named prototype/constructor installers,
//! dictionaries, and shared initialization fragments out of instance branding.
//! These fragments never erase an identity already present on their target.
//!
//! Register inheritance with `register_web_api_interfaces` using the same
//! interface metadata that drives constructor installation. `WebApiInterface`
//! registers its own name and parent automatically. Identity is independent of
//! realm exposure and mutable JavaScript prototypes. Use `web_api_object_type`
//! for the primary interface and `implements_interface` for inherited receiver
//! checks. Native factories can call `initialize_web_api_object` directly.
//!
//! The derive generates a Rust-side `new(...)` constructor by default. The
//! generated constructor takes every non-`()` declaration field as a named
//! argument and fills `()` declaration fields with `()`. If every field is
//! `()`, the generated constructor is `new()`. This keeps dynamic state
//! explicit while removing boilerplate such as `brand: ()` for fixed
//! initialized slots or accessor/method declaration fields. A field can declare
//! `#[webapi(constructor_default = expr)]`, or bare
//! `#[webapi(constructor_default)]` for `Default::default()`, to keep a
//! Rust-side default out of the generated constructor while still installing
//! the field normally; this is distinct from `init = ...`, which creates a
//! JavaScript/V8-side default for a `()` field. Use
//! `#[webapi(no_dynamic_constructor)]` when the declaration already has a
//! hand-written constructor with narrower semantics.
//!
//! # Receiver checks and Promise-returning members
//!
//! `receiver = path` on a method or `accessor_property` declares a native brand
//! predicate with signature `fn(&mut v8::PinScope, v8::Local<v8::Object>) -> bool`.
//! The generated callback checks it before running the implementation, throwing
//! `TypeError("Illegal invocation")` on failure. The predicate must inspect native
//! identity or private slots, not public constructors, prototypes, or properties;
//! it must not execute JavaScript or throw. This preserves cross-realm receivers
//! without accepting forged prototypes or Proxy wrappers.
//!
//! Struct-level `receiver = path` supplies the default for instance methods and
//! accessor properties in all three derives; a field can override it. Static
//! methods, data properties, and holder-based native data properties do not
//! inherit this policy. Already-built `getter_value` functions cannot use it.
//!
//! `returns_promise` on a method (including a static method) or accessor getter
//! converts synchronous exceptions from both the receiver check and the callback
//! into rejected Promises in the callback's realm. Successful return values are
//! unchanged, so cached Promise identity is preserved. An accessor setter still
//! throws synchronously. Callback data and native function descriptors are
//! unchanged: these adapters are Rust callbacks, not JavaScript wrappers.
//!
//! # Function-template declaration model
//!
//! A `#[derive(WebApiFunctionTemplate)]` struct describes a constructor-backed
//! V8 `FunctionTemplate`, its constructor-template static methods, and its
//! prototype-template methods. This is the declaration path for bootstrap
//! surfaces that install onto `FunctionTemplate` and
//! `FunctionTemplate::prototype_template()` instead of an already-created V8
//! object. Template methods are non-enumerable by default, and can opt into
//! `enumerable`, `readonly`, or `dont_delete`; struct-level
//! `#[webapi(enumerable)]` applies the WebIDL operation default to string-keyed
//! methods while leaving well-known symbols explicit. Template
//! `#[webapi(accessor_property)]` creates getter/setter `FunctionTemplate`
//! values through `ObjectTemplate::set_accessor_property`, so callbacks receive
//! the actual JavaScript receiver through `FunctionCallbackArguments::this()`.
//! `#[webapi(native_data_property)]` is the explicit spelling for the
//! holder-based `ObjectTemplate::set_native_data_property_with_configuration`
//! path and should be reserved for internal properties that intentionally
//! operate on the native holder.
//! `#[webapi(intrinsic_data_property = v8::Intrinsic::...)]` installs a
//! realm-correct V8 intrinsic directly on the prototype template. This
//! template-only path avoids reading mutable public builtins such as
//! `globalThis.Array.prototype` and supports the normal descriptor flags and
//! well-known symbol keys.
//! Struct-level
//! `#[webapi(intrinsic_prototype_parent = v8::Intrinsic::...)]` creates the
//! hidden parent template required by V8 and links the declared prototype to a
//! realm-correct intrinsic prototype. This is intended for WebIDL iterator
//! prototypes and special interfaces such as `DOMException`; it does not read
//! public constructors such as `globalThis.Error`.
//! WebIDL iterator templates can pair this with struct-level
//! `readonly_prototype` and `prototype_to_string_tag = "..."`; the latter
//! installs the read-only, non-enumerable `Symbol.toStringTag` required by the
//! iterator prototype algorithms.
//! `#[webapi(constant)]` installs WebIDL constants on both the constructor
//! template and prototype template.
//!
//! Global interface exposure is deliberately outside either derive. The
//! renderer's aggregate exposed-interface installer uses V8 lazy data
//! properties so one complete interface—not an individual declaration
//! fragment—owns first-read materialization.

extern crate self as moli_webapi_declare;

mod brand;
mod callback;
mod declaration;
mod error;
mod property;
mod prototype;
mod value;

pub mod __private;

pub use moli_webapi_declare_derive::{WebApiFunctionTemplate, WebApiInterface, WebApiObject};
pub use v8;

/// Adapts a manual native constructor to the same identity initialization used
/// by interface derives. The interface must be a native declaration literal,
/// never a name obtained from `new.target` or another JavaScript property.
#[macro_export]
macro_rules! web_api_constructor {
    ($interface:literal, $callback:path) => {{
        fn __moli_native_constructor_adapter<'s>(
            scope: &mut $crate::v8::PinScope<'s, '_>,
            args: $crate::v8::FunctionCallbackArguments<'s>,
            rv: $crate::v8::ReturnValue<'s>,
        ) {
            $crate::__private::invoke_web_api_constructor(scope, args, rv, $interface, $callback);
        }
        __moli_native_constructor_adapter
    }};
}

pub use brand::{
    WebApiType, implements_interface, initialize_web_api_object, register_web_api_interfaces,
    register_web_api_proxy, web_api_object_type,
};

pub use declaration::{
    DataPropertyDescriptorDeclaration, ObjectLiteralDeclaration, WebApiFunctionTemplateDeclaration,
    WebApiInterfaceDeclaration, WebApiObjectDeclaration,
};
pub use error::BindError;
pub use property::{
    define_bool_property, define_declared_accessor_property,
    define_declared_accessor_property_by_key, define_declared_constant_property,
    define_declared_data_property, define_declared_data_property_with_attributes,
    define_declared_enumerable_data_property, define_declared_hidden_property,
    define_declared_hidden_property_with_descriptor, define_declared_private_slot,
    define_enumerable_value_property, define_hidden_property, define_number_property,
    define_private_slot, define_string_property, define_value_property,
    illegal_constructor_callback, webidl_constant_property_attributes,
};
pub use prototype::{
    define_declared_to_string_tag, define_declared_to_string_tag_with_attributes,
    define_interface_constructor_property, define_interface_prototype_property,
    define_to_string_tag, define_to_string_tag_with_attributes, set_declared_prototype,
    set_interface_prototype, set_required_interface_prototype,
};
pub use value::{WebApiTemplateValue, WebApiValue, define_array_data_property};
