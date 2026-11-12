use super::super::*;
use super::live_dom::{LiveXPathResultNode, LiveXPathValue, evaluate_live_parsed_xpath};
use super::result::{
    XPathIteratorMutationState, build_xpath_nodes_result, build_xpath_scalar_result,
};
use super::snapshot::{build_xpath_snapshot, xpath_context_node_id};
use crate::native_bridge::document::{
    detached_node_type, detached_tree_query_version, detached_tree_root_object,
    live_get_attribute_node_ns_object, live_get_attribute_node_object,
};
use moli_xpath::{SnapshotValue, evaluate_parsed_snapshot_xpath};

use super::XPathEvaluationError;
use super::types::{XPATH_NUMBER_TYPE, XPATH_STRING_TYPE};

pub(super) fn evaluate_xpath_over_live_dom<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    runtime_ptr: *mut PageScriptHost,
    expression: &moli_xpath::Expression,
    context_handle: DomHandle,
    requested_result_type: u32,
) -> Result<Option<v8::Local<'s, v8::Object>>, XPathEvaluationError> {
    let (value, baseline_query_version) = {
        let runtime = unsafe { &*runtime_ptr };
        let value = evaluate_live_parsed_xpath(
            runtime.dom_host(),
            expression,
            context_handle,
            requested_result_type,
        )?;
        (value, runtime.dom_host().query_version())
    };

    match value {
        LiveXPathValue::Nodes(handles) => {
            let mut resolved = Vec::with_capacity(handles.len());
            for node in handles {
                let Some(node) = live_xpath_result_node_object(scope, runtime_ptr, node) else {
                    continue;
                };
                resolved.push(node);
            }
            Ok(build_xpath_nodes_result(
                scope,
                &resolved,
                requested_result_type,
                Some(XPathIteratorMutationState::Live {
                    runtime_ptr,
                    query_version: baseline_query_version,
                }),
            ))
        }
        LiveXPathValue::Scalar(value) => Ok(build_xpath_scalar_result(
            scope,
            value,
            requested_result_type,
        )),
    }
}

fn live_xpath_result_node_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    node: LiveXPathResultNode,
) -> Option<v8::Local<'s, v8::Object>> {
    match node {
        LiveXPathResultNode::Node(handle) => {
            let runtime = unsafe { &mut *runtime_ptr };
            runtime
                .native_bridge_mut()
                .wrap_handle(scope, runtime_ptr, handle)
        }
        LiveXPathResultNode::Attribute { owner, index } => {
            let (name, namespace_uri, local_name) = {
                let runtime = unsafe { &*runtime_ptr };
                let attribute = runtime
                    .dom_host()
                    .node(owner)?
                    .as_element()?
                    .attributes()
                    .get(index)?;
                (
                    attribute.name(),
                    (!attribute.namespace().is_empty()).then(|| attribute.namespace().to_owned()),
                    attribute.local_name().to_owned(),
                )
            };
            let runtime = unsafe { &mut *runtime_ptr };
            let owner = runtime
                .native_bridge_mut()
                .wrap_handle(scope, runtime_ptr, owner)?;
            if let Some(namespace_uri) = namespace_uri.as_deref() {
                live_get_attribute_node_ns_object(scope, owner, Some(namespace_uri), &local_name)
            } else {
                live_get_attribute_node_object(scope, owner, &name)
            }
        }
    }
}

pub(super) fn evaluate_xpath_over_object_tree<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    root: v8::Local<'s, v8::Object>,
    expression: &moli_xpath::Expression,
    context_node: Option<v8::Local<'s, v8::Object>>,
    requested_result_type: u32,
) -> Result<Option<v8::Local<'s, v8::Object>>, XPathEvaluationError> {
    let root_node_type = detached_node_type(scope, root).unwrap_or_default();
    if root_node_type != 9 && root_node_type != 1 {
        return Ok(None);
    }
    let iterator_mutation_state =
        object_tree_xpath_iterator_mutation_state(scope, context_node, root);

    let Some(snapshot) = build_xpath_snapshot(scope, root) else {
        return Ok(None);
    };
    let context = context_node.unwrap_or(root);
    let Some(context_id) = xpath_context_node_id(scope, context, &snapshot) else {
        return Ok(build_xpath_nodes_result(
            scope,
            &[],
            requested_result_type,
            iterator_mutation_state,
        ));
    };

    let value = evaluate_parsed_snapshot_xpath(&snapshot.snapshot, expression, context_id)
        .map_err(|_| XPathEvaluationError::InvalidExpression)?;

    match value {
        SnapshotValue::Nodes(nodes)
            if matches!(requested_result_type, XPATH_NUMBER_TYPE | XPATH_STRING_TYPE) =>
        {
            let text = nodes
                .first()
                .and_then(|id| snapshot.snapshot.node(*id))
                .map(|node| moli_xpath::Node::text_content(&node))
                .unwrap_or_default();
            Ok(build_xpath_scalar_result(
                scope,
                SnapshotValue::String(text),
                requested_result_type,
            ))
        }
        SnapshotValue::Nodes(nodes) => {
            let mut resolved = Vec::new();
            for node_id in nodes {
                if let Some(Some(original)) = snapshot.original_nodes.get(node_id) {
                    resolved.push(*original);
                }
            }
            Ok(build_xpath_nodes_result(
                scope,
                &resolved,
                requested_result_type,
                iterator_mutation_state,
            ))
        }
        value => Ok(build_xpath_scalar_result(
            scope,
            value,
            requested_result_type,
        )),
    }
}

fn object_tree_xpath_iterator_mutation_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context_node: Option<v8::Local<'s, v8::Object>>,
    root: v8::Local<'s, v8::Object>,
) -> Option<XPathIteratorMutationState<'s>> {
    let candidate = context_node.unwrap_or(root);
    let root = match detached_tree_root_object(scope, candidate) {
        Some(root) => root,
        None => detached_tree_root_object(scope, root)?,
    };
    let query_version = detached_tree_query_version(scope, root)?;
    Some(XPathIteratorMutationState::ObjectTree {
        root,
        query_version,
    })
}
