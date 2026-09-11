use super::callbacks::{
    url_search_params_append_callback, url_search_params_constructor_callback,
    url_search_params_delete_callback, url_search_params_entries_callback,
    url_search_params_for_each_callback, url_search_params_get_all_callback,
    url_search_params_get_callback, url_search_params_has_callback,
    url_search_params_keys_callback, url_search_params_set_callback,
    url_search_params_sort_callback, url_search_params_to_string_callback,
    url_search_params_values_callback,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, v8};

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::URLSearchParams,
    constructor_callback = url_search_params_constructor_callback,
    constructor_length = 0,
    enumerable
)]
struct UrlSearchParamsTemplateDeclaration {
    #[webapi(method, length = 1, callback = url_search_params_has_callback)]
    has: (),

    #[webapi(method, length = 1, callback = url_search_params_get_callback)]
    get: (),

    #[webapi(method, length = 1, callback = url_search_params_get_all_callback)]
    get_all: (),

    #[webapi(method, length = 2, callback = url_search_params_append_callback)]
    append: (),

    #[webapi(method, length = 2, callback = url_search_params_set_callback)]
    set: (),

    #[webapi(method, length = 1, callback = url_search_params_delete_callback)]
    r#delete: (),

    #[webapi(method, length = 0, callback = url_search_params_keys_callback)]
    keys: (),

    #[webapi(method, length = 0, callback = url_search_params_values_callback)]
    values: (),

    #[webapi(method, length = 0, callback = url_search_params_entries_callback)]
    entries: (),

    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),

    #[webapi(method, length = 1, callback = url_search_params_for_each_callback)]
    for_each: (),

    #[webapi(method, length = 0, callback = url_search_params_sort_callback)]
    sort: (),

    #[webapi(method, length = 0, callback = url_search_params_to_string_callback)]
    to_string: (),
}

pub(in crate::context_bootstrap) fn build_url_search_params_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = UrlSearchParamsTemplateDeclaration::build(scope);
    super::UrlSearchParamsPrototypeAccessorsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
    template
}
