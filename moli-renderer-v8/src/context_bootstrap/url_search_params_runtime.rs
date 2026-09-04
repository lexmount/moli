use super::form_data_runtime::{form_data_entries, form_data_is_object};
use super::url_form::{
    apply_url_update, callback_arg_url_like_string, callback_value_string, url_object_value,
};
use super::*;
use crate::util::{get_private_object, get_private_value, set_private_value};
use moli_webapi_declare::WebApiFunctionTemplate;

mod callbacks;
mod iterators;
mod parse;
mod storage;
mod template;

pub(super) use moli_url::search_params::url_query_pairs;
pub(super) use storage::new_url_search_params_object;
pub(crate) use storage::url_search_params_request_body;
pub(super) use template::build_url_search_params_constructor_template;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "URLSearchParams", enumerable)]
struct UrlSearchParamsPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = callbacks::url_search_params_size_getter_callback, enumerable)]
    size: (),
}
