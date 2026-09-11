use super::attributes::initialize_url_prototype_accessors;
use super::callbacks::{
    url_can_parse_callback, url_constructor_callback, url_create_object_url_callback,
    url_parse_callback, url_revoke_object_url_callback, url_to_json_callback,
    url_to_string_callback,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, v8};

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::URL,
    constructor_callback = url_constructor_callback,
    constructor_length = 1,
    enumerable
)]
struct UrlTemplateDeclaration {
    #[webapi(static_method, length = 1, callback = url_parse_callback)]
    parse: (),

    #[webapi(static_method, length = 1, callback = url_can_parse_callback)]
    can_parse: (),

    #[webapi(
        static_method = "createObjectURL",
        length = 1,
        callback = url_create_object_url_callback
    )]
    create_object_url: (),

    #[webapi(
        static_method = "revokeObjectURL",
        length = 1,
        callback = url_revoke_object_url_callback
    )]
    revoke_object_url: (),

    #[webapi(method, length = 0, callback = url_to_string_callback)]
    to_string: (),

    #[webapi(method = "toJSON", length = 0, callback = url_to_json_callback)]
    to_json: (),
}

pub(in crate::context_bootstrap) fn build_url_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = UrlTemplateDeclaration::build(scope);
    initialize_url_prototype_accessors(scope, template.prototype_template(scope));
    template
}
