use super::url_search_params::{new_url_search_params_object, url_query_pairs};
use moli_v8_util::{throw_type_error, v8_string};

mod attributes;
mod callbacks;
mod helpers;
mod template;

pub use helpers::callback_value_string;
pub(crate) use helpers::{apply_url_update, url_object_value};
pub use template::build_url_constructor_template;

const URL_HREF_SLOT: &str = "__moliUrlHref";
const URL_SEARCH_PARAMS_SLOT: &str = "__moliUrlSearchParams";
