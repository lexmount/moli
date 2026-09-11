mod attrs;
mod expand;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Derives a Web API function-template declaration binder.
///
/// The generated implementation creates a V8 `FunctionTemplate`, sets its class
/// name, and installs declared constructor-template static methods and
/// prototype-template methods and intrinsic data properties.
/// `#[webapi(intrinsic_prototype_parent = v8::Intrinsic::...)]` links the
/// generated prototype to a realm-correct V8 intrinsic without reading public
/// JavaScript constructors.
/// Iterator prototypes can additionally declare `readonly_prototype` and
/// `prototype_to_string_tag = "..."` to match the WebIDL iterator template
/// shape.
/// `#[webapi(constant)]` fields are installed as WebIDL constants on both the
/// constructor template and prototype template.
/// This is the template path for constructor-backed Web API surfaces that are
/// not instantiated as plain V8 objects during bootstrap.
#[proc_macro_derive(WebApiFunctionTemplate, attributes(webapi))]
pub fn derive_webapi_function_template(input: TokenStream) -> TokenStream {
    match expand::expand_webapi_function_template(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Derives a Web API object declaration binder.
///
/// Choose `interface = NativeType` for native identity, `record` for a plain
/// object, or `fragment` for shared initialization without implicit identity or
/// prototype changes. Empty declarations and a single scope lifetime require
/// no additional flags.
///
/// Each annotated field selects one installation path: JavaScript own data
/// property, WebIDL constant, method, JavaScript accessor property, V8 native
/// data property, non-enumerable hidden property, V8 private slot, runtime
/// prototype, or runtime toStringTag. A struct-level
/// `#[webapi(data_properties)]` turns otherwise unannotated fields into
/// JavaScript data properties. Without that struct-level default, unannotated
/// fields are declaration-only inputs: the generated code does not install
/// them, but method attributes such as `data = self.state` can still reference
/// them while building callback functions.
#[proc_macro_derive(WebApiObject, attributes(webapi))]
pub fn derive_webapi_object(input: TokenStream) -> TokenStream {
    match expand::expand_webapi_object(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}
