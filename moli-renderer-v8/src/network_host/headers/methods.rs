mod access;
mod iteration;
mod mutation;

use super::*;
use crate::web_api_interfaces;

pub(super) use self::access::{
    headers_get_callback, headers_get_set_cookie_callback, headers_has_callback,
};
pub(super) use self::iteration::{
    headers_entries_callback, headers_for_each_callback, headers_keys_callback,
    headers_values_callback,
};
pub(super) use self::mutation::{
    headers_append_callback, headers_delete_callback, headers_set_callback,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

pub(super) fn require_headers_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if web_api_interfaces::Headers::is_instance(scope, receiver) {
        return Some(receiver);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::Headers)]
struct HeadersObjectMethodsDeclaration {
    #[webapi(method, enumerable, length = 1, callback = headers_get_callback)]
    get: (),
    #[webapi(method, enumerable, length = 1, callback = headers_has_callback)]
    has: (),
    #[webapi(method, enumerable, length = 0, callback = headers_get_set_cookie_callback)]
    get_set_cookie: (),
    #[webapi(method, enumerable, length = 2, callback = headers_set_callback)]
    set: (),
    #[webapi(method, enumerable, length = 1, callback = headers_delete_callback)]
    delete: (),
    #[webapi(method, enumerable, length = 2, callback = headers_append_callback)]
    append: (),
    #[webapi(method, enumerable, length = 0, callback = headers_keys_callback)]
    keys: (),
    #[webapi(method, enumerable, length = 0, callback = headers_values_callback)]
    values: (),
    #[webapi(method, enumerable, length = 0, callback = headers_entries_callback)]
    entries: (),
    #[webapi(alias = "entries", symbol = "iterator", enumerable)]
    iterator: (),
    #[webapi(method, enumerable, length = 1, callback = headers_for_each_callback)]
    for_each: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Headers, enumerable)]
struct HeadersPrototypeMethodsDeclaration {
    #[webapi(method, length = 1, callback = headers_get_callback)]
    get: (),
    #[webapi(method, length = 1, callback = headers_has_callback)]
    has: (),
    #[webapi(method, length = 0, callback = headers_get_set_cookie_callback)]
    get_set_cookie: (),
    #[webapi(method, length = 2, callback = headers_set_callback)]
    set: (),
    #[webapi(method, length = 1, callback = headers_delete_callback)]
    delete: (),
    #[webapi(method, length = 2, callback = headers_append_callback)]
    append: (),
    #[webapi(method, length = 0, callback = headers_keys_callback)]
    keys: (),
    #[webapi(method, length = 0, callback = headers_values_callback)]
    values: (),
    #[webapi(method, length = 0, callback = headers_entries_callback)]
    entries: (),
    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),
    #[webapi(method, length = 1, callback = headers_for_each_callback)]
    for_each: (),
}

pub(in crate::network_host) fn install_headers_object_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    headers_obj: v8::Local<'s, v8::Object>,
) {
    HeadersObjectMethodsDeclaration::default()
        .initialize(scope, headers_obj)
        .expect("Headers object methods declaration should initialize");
}

pub(crate) fn install_headers_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    HeadersPrototypeMethodsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}
