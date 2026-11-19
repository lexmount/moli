//! XPath result objects initialized before the evaluator installs results and iterator state.

use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::WebApiObject;

pub const XPATH_RESULT_TYPE_SLOT: &str = "__moliXPathType";
pub const XPATH_RESULT_STRING_VALUE_SLOT: &str = "__moliXPathStringValue";
pub const XPATH_RESULT_NUMBER_VALUE_SLOT: &str = "__moliXPathNumberValue";
pub const XPATH_RESULT_BOOLEAN_VALUE_SLOT: &str = "__moliXPathBooleanValue";
pub const XPATH_RESULT_SINGLE_NODE_VALUE_SLOT: &str = "__moliXPathSingleNodeValue";
pub const XPATH_RESULT_SNAPSHOT_LENGTH_SLOT: &str = "__moliXPathSnapshotLength";
pub const XPATH_RESULT_INDEX_SLOT: &str = "__moliXPathIndex";
pub const XPATH_RESULT_NODES_SLOT: &str = "__moliXPathNodes";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::XPathResult)]
struct XPathResultBaseDeclaration<'scope> {
    #[webapi(slot = XPATH_RESULT_TYPE_SLOT)]
    result_type: u32,
    #[webapi(slot = XPATH_RESULT_STRING_VALUE_SLOT, constructor_default = "")]
    string_value: &'static str,
    #[webapi(slot = XPATH_RESULT_NUMBER_VALUE_SLOT, constructor_default = 0.0)]
    number_value: f64,
    #[webapi(slot = XPATH_RESULT_BOOLEAN_VALUE_SLOT, constructor_default = false)]
    boolean_value: bool,
    #[webapi(slot = XPATH_RESULT_SINGLE_NODE_VALUE_SLOT, init = "null")]
    _single_node_value: (),
    #[webapi(slot = XPATH_RESULT_SNAPSHOT_LENGTH_SLOT, constructor_default = 0)]
    snapshot_length: u32,
    #[webapi(slot = XPATH_RESULT_INDEX_SLOT, constructor_default = 0)]
    index: i32,
    #[webapi(slot = XPATH_RESULT_NODES_SLOT, constructor_default = Vec::new())]
    nodes: Vec<v8::Local<'scope, v8::Value>>,
}

pub fn build_xpath_result_base<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result_type: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    XPathResultBaseDeclaration::new(result_type)
        .bind(scope)
        .ok()
}
