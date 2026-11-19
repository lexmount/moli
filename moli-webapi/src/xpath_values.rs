//! XPath result objects initialized before the evaluator installs results and iterator state.

use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

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

pub const XPATH_ANY_TYPE: u32 = 0;
pub const XPATH_NUMBER_TYPE: u32 = 1;
pub const XPATH_STRING_TYPE: u32 = 2;
pub const XPATH_BOOLEAN_TYPE: u32 = 3;
pub const XPATH_UNORDERED_NODE_ITERATOR_TYPE: u32 = 4;
pub const XPATH_ORDERED_NODE_ITERATOR_TYPE: u32 = 5;
pub const XPATH_UNORDERED_NODE_SNAPSHOT_TYPE: u32 = 6;
pub const XPATH_ORDERED_NODE_SNAPSHOT_TYPE: u32 = 7;
pub const XPATH_ANY_UNORDERED_NODE_TYPE: u32 = 8;
pub const XPATH_FIRST_ORDERED_NODE_TYPE: u32 = 9;
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XPathResult)]
pub struct XPathResultConstantsDeclaration {
    #[webapi(constant = "ANY_TYPE", value = XPATH_ANY_TYPE)]
    any_type: (),

    #[webapi(constant = "NUMBER_TYPE", value = XPATH_NUMBER_TYPE)]
    number_type: (),

    #[webapi(constant = "STRING_TYPE", value = XPATH_STRING_TYPE)]
    string_type: (),

    #[webapi(constant = "BOOLEAN_TYPE", value = XPATH_BOOLEAN_TYPE)]
    boolean_type: (),

    #[webapi(
        constant = "UNORDERED_NODE_ITERATOR_TYPE",
        value = XPATH_UNORDERED_NODE_ITERATOR_TYPE
    )]
    unordered_node_iterator_type: (),

    #[webapi(
        constant = "ORDERED_NODE_ITERATOR_TYPE",
        value = XPATH_ORDERED_NODE_ITERATOR_TYPE
    )]
    ordered_node_iterator_type: (),

    #[webapi(
        constant = "UNORDERED_NODE_SNAPSHOT_TYPE",
        value = XPATH_UNORDERED_NODE_SNAPSHOT_TYPE
    )]
    unordered_node_snapshot_type: (),

    #[webapi(
        constant = "ORDERED_NODE_SNAPSHOT_TYPE",
        value = XPATH_ORDERED_NODE_SNAPSHOT_TYPE
    )]
    ordered_node_snapshot_type: (),

    #[webapi(
        constant = "ANY_UNORDERED_NODE_TYPE",
        value = XPATH_ANY_UNORDERED_NODE_TYPE
    )]
    any_unordered_node_type: (),

    #[webapi(
        constant = "FIRST_ORDERED_NODE_TYPE",
        value = XPATH_FIRST_ORDERED_NODE_TYPE
    )]
    first_ordered_node_type: (),
}
