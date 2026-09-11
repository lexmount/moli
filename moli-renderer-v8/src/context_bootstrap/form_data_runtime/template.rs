use super::callbacks::{
    form_data_append_callback, form_data_constructor_callback, form_data_delete_callback,
    form_data_entries_callback, form_data_for_each_callback, form_data_get_all_callback,
    form_data_get_callback, form_data_has_callback, form_data_keys_callback,
    form_data_set_callback, form_data_values_callback,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, v8};

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::FormData,
    constructor_callback = form_data_constructor_callback,
    constructor_length = 0,
    enumerable
)]
struct FormDataTemplateDeclaration {
    #[webapi(method, length = 2, callback = form_data_append_callback)]
    append: (),

    #[webapi(method, length = 2, callback = form_data_set_callback)]
    set: (),

    #[webapi(method, length = 1, callback = form_data_get_callback)]
    get: (),

    #[webapi(method, length = 1, callback = form_data_get_all_callback)]
    get_all: (),

    #[webapi(method, length = 1, callback = form_data_has_callback)]
    has: (),

    #[webapi(method, length = 1, callback = form_data_delete_callback)]
    r#delete: (),

    #[webapi(method, length = 0, callback = form_data_keys_callback)]
    keys: (),

    #[webapi(method, length = 0, callback = form_data_values_callback)]
    values: (),

    #[webapi(method, length = 0, callback = form_data_entries_callback)]
    entries: (),

    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),

    #[webapi(method, length = 1, callback = form_data_for_each_callback)]
    for_each: (),
}

pub(in crate::context_bootstrap) fn build_form_data_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    FormDataTemplateDeclaration::build(scope)
}
