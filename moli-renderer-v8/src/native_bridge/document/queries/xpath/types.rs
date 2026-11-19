pub(super) use moli_webapi::xpath_values::{
    XPATH_RESULT_BOOLEAN_VALUE_SLOT, XPATH_RESULT_INDEX_SLOT, XPATH_RESULT_NODES_SLOT,
    XPATH_RESULT_NUMBER_VALUE_SLOT, XPATH_RESULT_SINGLE_NODE_VALUE_SLOT,
    XPATH_RESULT_SNAPSHOT_LENGTH_SLOT, XPATH_RESULT_STRING_VALUE_SLOT, XPATH_RESULT_TYPE_SLOT,
};
pub(super) const XPATH_RESULT_RUNTIME_SLOT: &str = "__moliXPathRuntime";
pub(super) const XPATH_RESULT_OBJECT_TREE_ROOT_SLOT: &str = "__moliXPathObjectTreeRoot";
pub(super) const XPATH_RESULT_QUERY_VERSION_SLOT: &str = "__moliXPathQueryVersion";

pub(super) const XPATH_ANY_TYPE: u32 = 0;
pub(super) const XPATH_NUMBER_TYPE: u32 = 1;
pub(super) const XPATH_STRING_TYPE: u32 = 2;
pub(super) const XPATH_BOOLEAN_TYPE: u32 = 3;
pub(super) const XPATH_UNORDERED_NODE_ITERATOR_TYPE: u32 = 4;
pub(super) const XPATH_ORDERED_NODE_ITERATOR_TYPE: u32 = 5;
pub(super) const XPATH_UNORDERED_NODE_SNAPSHOT_TYPE: u32 = 6;
pub(super) const XPATH_ORDERED_NODE_SNAPSHOT_TYPE: u32 = 7;
pub(super) const XPATH_ANY_UNORDERED_NODE_TYPE: u32 = 8;
pub(super) const XPATH_FIRST_ORDERED_NODE_TYPE: u32 = 9;
