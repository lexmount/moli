use super::url_api::{apply_url_update, callback_value_string, url_object_value};
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_object, get_private_value, set_private_value};
use moli_v8_util::{throw_type_error, v8_string};
use moli_webapi_declare::WebApiFunctionTemplate;

mod callbacks;
mod iterators;
mod parse;
mod storage;
mod template;

pub(super) use moli_url::search_params::url_query_pairs;
pub(crate) use storage::new_url_search_params_object;
pub use storage::url_search_params_request_body;
pub use template::build_url_search_params_constructor_template;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::URLSearchParams, enumerable)]
struct UrlSearchParamsPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = callbacks::url_search_params_size_getter_callback, enumerable)]
    size: (),
}

const URL_SEARCH_PARAMS_OWNER_SLOT: &str = "__moliUrlSearchParamsOwner";
const URL_SEARCH_PARAMS_PAIRS_SLOT: &str = "__moliUrlSearchParamsPairs";
