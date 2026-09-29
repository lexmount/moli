use crate::dom::native::{Node, NodeType};

use super::*;

mod child_node;
mod core;
mod fragment;
mod parent_node;

pub(in crate::native_bridge) use child_node::{
    ChildNodeMutation, mutate_child_node, node_after_callback, node_before_callback,
    node_remove_callback, node_replace_with_callback,
};
pub(crate) use core::validate_pre_insert_handles;
pub(in crate::native_bridge) use core::{
    node_append_child_callback, node_insert_before_callback, node_move_before_callback,
    node_remove_child_callback, node_replace_child_callback, replace_node_child,
};
pub(in crate::native_bridge) use parent_node::{
    ParentNodeMutation, mutate_parent_node, node_append_callback, node_prepend_callback,
    node_replace_children_callback,
};
