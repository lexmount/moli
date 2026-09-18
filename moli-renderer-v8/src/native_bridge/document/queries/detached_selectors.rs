use std::collections::VecDeque;

use percent_encoding::percent_decode_str;
use style::context::QuirksMode;
use url::Url;

use super::super::{XHTML_NS, normalize_namespace};
use super::*;
use crate::native_bridge::document::{
    detached_character_data_value, detached_child_node_objects, detached_document_state_string,
    detached_element_local_name, detached_element_namespace_uri, detached_is_node,
    detached_live_delegate_object, detached_native_child_node_objects,
    detached_native_handle_for_runtime, detached_native_object_for_handle, detached_node_type,
    detached_owner_document_object, detached_parent_node_object,
};
use crate::native_bridge::node::throw_native_selector_error_for_selector;
use crate::native_bridge::{
    collections,
    identity::{CollectionKind, LiveCollectionQueryKind},
    node::node_runtime_and_handle_from_object_or_detached,
};
use crate::util::context_host_ptr_from_global_bridge;
use crate::webidl;
use crate::{
    custom_elements,
    dom::{custom_elements::is_valid_custom_element_name, native::CustomElementState},
};
use moli_selector::{
    DetachedStyloSelectorHost, detached_stylo_selector_matches,
    detached_stylo_selector_matches_if_uses_defined_pseudo, detached_stylo_selector_query_all,
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.getElementById")]
struct DetachedDocumentGetElementByIdArgs {
    #[webidl(index = 1, required)]
    element_id: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.querySelector")]
struct DetachedDocumentQuerySelectorArgs {
    #[webidl(index = 1, required)]
    selectors: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.querySelectorAll")]
struct DetachedDocumentQuerySelectorAllArgs {
    #[webidl(index = 1, required)]
    selectors: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.matches")]
struct DetachedElementMatchesArgs {
    #[webidl(index = 1, required)]
    selectors: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.getElementsByTagName")]
struct DetachedDocumentGetElementsByTagNameArgs {
    #[webidl(index = 1, required)]
    qualified_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.getElementsByTagNameNS")]
struct DetachedDocumentGetElementsByTagNameNsArgs {
    #[webidl(index = 1, required, nullable)]
    namespace: Option<String>,
    #[webidl(index = 2, required)]
    local_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.getElementsByClassName")]
struct DetachedDocumentGetElementsByClassNameArgs {
    #[webidl(index = 1, required)]
    class_names: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.getElementsByName")]
struct DetachedDocumentGetElementsByNameArgs {
    #[webidl(index = 1, required)]
    element_name: String,
}

struct V8DetachedSelectorHost<'scope, 'pin, 'host> {
    scope: &'host mut v8::PinScope<'scope, 'pin>,
}

impl<'scope, 'pin, 'host> DetachedStyloSelectorHost
    for V8DetachedSelectorHost<'scope, 'pin, 'host>
{
    type Node = v8::Local<'scope, v8::Object>;

    fn same_node(&mut self, a: Self::Node, b: Self::Node) -> bool {
        a.strict_equals(b.into())
    }

    fn node_type(&mut self, node: Self::Node) -> Option<i32> {
        detached_selector_node_type(self.scope, node)
    }

    fn child_nodes(&mut self, node: Self::Node) -> Vec<Self::Node> {
        detached_child_node_objects(self.scope, node)
    }

    fn structural_child_nodes(&mut self, node: Self::Node) -> Option<Vec<Self::Node>> {
        detached_native_child_node_objects(self.scope, node)
    }

    fn parent_node(&mut self, node: Self::Node) -> Option<Self::Node> {
        detached_parent_node_object(self.scope, node)
    }

    fn node_value(&mut self, node: Self::Node) -> Option<String> {
        detached_selector_node_value(self.scope, node)
    }

    fn attribute_value(&mut self, node: Self::Node, name: &str) -> Option<String> {
        detached_element_attribute_value(self.scope, node, name)
    }

    fn local_name(&mut self, node: Self::Node) -> Option<String> {
        detached_element_local_name_value(self.scope, node)
    }

    fn namespace_uri(&mut self, node: Self::Node) -> Option<String> {
        detached_element_namespace_value(self.scope, node)
    }

    fn document_url(&mut self, root: Self::Node) -> Option<Url> {
        let document = detached_query_document_for_root(self.scope, root);
        let url = detached_document_state_string(self.scope, document, "url", "about:blank");
        Url::parse(&url).ok()
    }

    fn quirks_mode(&mut self, root: Self::Node) -> QuirksMode {
        let document = detached_query_document_for_root(self.scope, root);
        match detached_document_state_string(self.scope, document, "compatMode", "CSS1Compat")
            .as_str()
        {
            "BackCompat" => QuirksMode::Quirks,
            _ => QuirksMode::NoQuirks,
        }
    }

    fn matches_target_pseudo(&mut self, node: Self::Node, tree_root: Self::Node) -> bool {
        detached_query_matches_target_pseudo(self.scope, node, tree_root)
    }

    fn matches_defined_pseudo(&mut self, node: Self::Node) -> bool {
        detached_query_matches_defined_pseudo(self.scope, node)
    }
}

fn detached_query_document_for_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if detached_selector_node_type(scope, root) == Some(9) {
        root
    } else {
        detached_owner_document_object(scope, root).unwrap_or(root)
    }
}

fn detached_query_matches_defined_pseudo<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, node)
        && let Some(element) = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(crate::dom::native::Node::as_element)
    {
        if element.namespace() != XHTML_NS {
            return true;
        }
        return match element.custom_element_state() {
            CustomElementState::Custom => true,
            CustomElementState::Undefined | CustomElementState::Failed => false,
            CustomElementState::Uncustomized => !is_valid_custom_element_name(element.local_name()),
        };
    }

    if detached_element_namespace_value(scope, node).as_deref() != Some(XHTML_NS) {
        return true;
    }
    let Some(local_name) = detached_element_local_name_value(scope, node) else {
        return true;
    };
    let is_name = detached_element_attribute_value(scope, node, "is");
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return !is_valid_custom_element_name(&local_name) && is_name.is_none();
    };
    let Some(registry) = object_property_as_object(scope, node, "customElementRegistry") else {
        return !is_valid_custom_element_name(&local_name) && is_name.is_none();
    };
    let registry_key = custom_elements::registry_store_key(scope, registry);
    let store = unsafe { &*runtime_ptr }.custom_elements_for_registry_key(registry_key);
    if let Some(is_name) = is_name.as_deref() {
        return store
            .and_then(|store| store.definition_extends_local_name(is_name))
            .is_some_and(|extends_local_name| extends_local_name == local_name);
    }
    if is_valid_custom_element_name(&local_name) {
        return store.is_some_and(|store| store.has_autonomous_definition(&local_name));
    }
    true
}

fn detached_query_matches_target_pseudo<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    tree_root: v8::Local<'s, v8::Object>,
) -> bool {
    let connected_to_document = detached_selector_node_type(scope, tree_root) == Some(9)
        || detached_native_node_is_connected(scope, node);
    if !connected_to_document {
        return false;
    }
    let document = if detached_selector_node_type(scope, tree_root) == Some(9) {
        tree_root
    } else {
        detached_owner_document_object(scope, node).unwrap_or(tree_root)
    };
    let url = detached_document_state_string(scope, document, "url", "about:blank");
    let Some(fragment) = url::Url::parse(&url).ok().and_then(|url| {
        url.fragment().map(|fragment| {
            percent_decode_str(fragment)
                .decode_utf8_lossy()
                .into_owned()
        })
    }) else {
        return false;
    };
    !fragment.is_empty()
        && detached_element_attribute_value(scope, node, "id").as_deref() == Some(fragment.as_str())
}

fn detached_native_node_is_connected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    detached_native_handle_for_runtime(scope, runtime_ptr, node)
        .is_some_and(|handle| unsafe { &*runtime_ptr }.dom_host().is_connected(handle))
}

fn detached_query_roots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    match detached_selector_node_type(scope, root) {
        Some(9) => detached_child_node_objects(scope, root)
            .into_iter()
            .find(|child| detached_selector_node_type(scope, *child) == Some(1))
            .into_iter()
            .collect(),
        _ => detached_child_node_objects(scope, root),
    }
}

fn detached_query_child_nodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    detached_child_node_objects(scope, node)
}

pub(in crate::native_bridge::document) fn detached_query_selector_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    selector: &str,
    find_all: bool,
) -> Vec<v8::Local<'s, v8::Object>> {
    detached_query_selector_objects_result(scope, root, selector, find_all).unwrap_or_default()
}

fn detached_query_selector_objects_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    selector: &str,
    find_all: bool,
) -> Result<Vec<v8::Local<'s, v8::Object>>, crate::selector::SelectorError> {
    let normalized = selector.trim();
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(result) = detached_native_query_selector_objects(scope, root, normalized, find_all)
    {
        return result;
    }

    let mut host = V8DetachedSelectorHost { scope };
    detached_stylo_selector_query_all(&mut host, root, normalized, find_all)
}

fn detached_stylo_matches_selector<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    selector: &str,
) -> Result<bool, crate::selector::SelectorError> {
    let mut host = V8DetachedSelectorHost { scope };
    detached_stylo_selector_matches(&mut host, node, selector)
}

fn detached_stylo_matches_selector_if_uses_defined_pseudo<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    selector: &str,
) -> Result<Option<bool>, crate::selector::SelectorError> {
    let mut host = V8DetachedSelectorHost { scope };
    detached_stylo_selector_matches_if_uses_defined_pseudo(&mut host, node, selector)
}

fn detached_native_query_selector_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    selector: &str,
    find_all: bool,
) -> Option<Result<Vec<v8::Local<'s, v8::Object>>, crate::selector::SelectorError>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let root_handle = detached_native_handle_for_runtime(scope, runtime_ptr, root)?;
    let handles = if find_all {
        unsafe { &*runtime_ptr }.query_selector_all(Some(root_handle), selector)
    } else {
        unsafe { &*runtime_ptr }
            .query_selector(Some(root_handle), selector)
            .map(|handle| handle.into_iter().collect())
    };
    Some(handles.map(|handles: Vec<DomHandle>| {
        handles
            .into_iter()
            .filter_map(|handle| detached_native_object_for_handle(scope, runtime_ptr, handle))
            .collect()
    }))
}

fn detached_native_query_selector_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    selector: &str,
) -> Option<Result<Vec<DomHandle>, crate::selector::SelectorError>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let root_handle = detached_native_handle_for_runtime(scope, runtime_ptr, root)?;
    Some(unsafe { &*runtime_ptr }.query_selector_all(Some(root_handle), selector))
}

fn detached_native_matches_selector<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    selector: &str,
) -> Option<Result<bool, crate::selector::SelectorError>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    Some(unsafe { &*runtime_ptr }.matches(handle, selector))
}

fn detached_native_element_by_id_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    id: &str,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let root_handle = detached_native_handle_for_runtime(scope, runtime_ptr, root)?;
    let found = unsafe { &*runtime_ptr }
        .dom_host()
        .element_handle_by_id_in_subtree(root_handle, id);
    Some(found.and_then(|handle| detached_native_object_for_handle(scope, runtime_ptr, handle)))
}

fn detached_native_element_query_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    query: impl FnOnce(&crate::dom::native::DomHost, DomHandle, bool) -> Vec<DomHandle>,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let root_handle = detached_native_handle_for_runtime(scope, runtime_ptr, root)?;
    let handles = {
        let dom_host = unsafe { &*runtime_ptr }.dom_host();
        let include_root = dom_host
            .node(root_handle)
            .is_some_and(crate::dom::native::Node::is_document);
        query(dom_host, root_handle, include_root)
    };
    Some(
        handles
            .into_iter()
            .filter_map(|handle| detached_native_object_for_handle(scope, runtime_ptr, handle))
            .collect(),
    )
}

fn detached_element_matches_tag_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    qualified_name: &str,
) -> bool {
    if qualified_name == "*" {
        return true;
    }
    let Some(local_name) = detached_element_local_name_value(scope, node) else {
        return false;
    };
    match detached_element_namespace_value(scope, node).as_deref() {
        Some(namespace) if namespace == XHTML_NS => {
            local_name == qualified_name.to_ascii_lowercase()
        }
        _ => local_name == qualified_name,
    }
}

fn detached_element_matches_tag_name_ns<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    local_name: &str,
) -> bool {
    let actual_namespace = detached_element_namespace_value(scope, node);
    let namespace_matches = match namespace {
        Some("*") => true,
        Some(expected) => actual_namespace.as_deref() == Some(expected),
        None => actual_namespace.is_none(),
    };
    let local_name_matches = local_name == "*"
        || detached_element_local_name_value(scope, node)
            .is_some_and(|actual| actual == local_name);
    namespace_matches && local_name_matches
}

fn collect_detached_elements<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    mut matches: impl FnMut(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>) -> bool,
) -> Vec<v8::Local<'s, v8::Object>> {
    let mut queue: VecDeque<v8::Local<'s, v8::Object>> =
        detached_query_roots(scope, root).into_iter().collect();
    let mut found = Vec::new();
    while let Some(node) = queue.pop_front() {
        if detached_selector_node_type(scope, node) == Some(1) && matches(scope, node) {
            found.push(node);
        }
        for child in detached_query_child_nodes(scope, node).into_iter().rev() {
            queue.push_front(child);
        }
    }
    found
}

pub(in crate::native_bridge) fn bridge_detached_get_element_by_id_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentGetElementByIdArgs>(scope, &args)
    else {
        return;
    };
    if parsed.element_id.is_empty() {
        rv.set_null();
        return;
    }
    if let Some(result) = detached_native_element_by_id_object(scope, root, &parsed.element_id) {
        match result {
            Some(node) => rv.set(node.into()),
            None => rv.set_null(),
        }
        return;
    }
    let selector = format!("#{}", parsed.element_id);
    match detached_query_selector_objects(scope, root, &selector, false)
        .into_iter()
        .next()
    {
        Some(node) => rv.set(node.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_query_selector_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentQuerySelectorArgs>(scope, &args) else {
        return;
    };
    if let Some(result) =
        detached_native_query_selector_objects(scope, root, &parsed.selectors, false)
    {
        match result {
            Ok(nodes) => match nodes.into_iter().next() {
                Some(node) => rv.set(node.into()),
                None => rv.set_null(),
            },
            Err(error) => {
                throw_native_selector_error_for_selector(scope, &parsed.selectors, &error)
            }
        }
        return;
    }
    match detached_query_selector_objects_result(scope, root, &parsed.selectors, false) {
        Ok(nodes) => match nodes.into_iter().next() {
            Some(node) => rv.set(node.into()),
            None => rv.set_null(),
        },
        Err(error) => throw_native_selector_error_for_selector(scope, &parsed.selectors, &error),
    }
}

pub(in crate::native_bridge) fn bridge_detached_query_selector_all_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentQuerySelectorAllArgs>(scope, &args)
    else {
        return;
    };
    if let Some(result) = detached_native_query_selector_handles(scope, root, &parsed.selectors) {
        match result {
            Ok(handles) => {
                match build_detached_native_node_list(scope, &handles) {
                    Some(list) => rv.set(list.into()),
                    None => rv.set_null(),
                }
                return;
            }
            Err(error) => {
                throw_native_selector_error_for_selector(scope, &parsed.selectors, &error);
                return;
            }
        }
    }
    let nodes = if let Some(result) =
        detached_native_query_selector_objects(scope, root, &parsed.selectors, true)
    {
        match result {
            Ok(nodes) => nodes,
            Err(error) => {
                throw_native_selector_error_for_selector(scope, &parsed.selectors, &error);
                return;
            }
        }
    } else {
        match detached_query_selector_objects_result(scope, root, &parsed.selectors, true) {
            Ok(nodes) => nodes,
            Err(error) => {
                throw_native_selector_error_for_selector(scope, &parsed.selectors, &error);
                return;
            }
        }
    };
    match build_detached_node_list(scope, &nodes) {
        Some(list) => rv.set(list.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_matches_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(node) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_bool(false);
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedElementMatchesArgs>(scope, &args) else {
        return;
    };
    if let Some(delegate) = detached_live_delegate_object(scope, node)
        && let Some(selector) = v8_string(scope, &parsed.selectors)
        && let Some(value) = call_object_method(scope, delegate, "matches", &[selector.into()])
    {
        rv.set(value);
        return;
    }
    match detached_stylo_matches_selector_if_uses_defined_pseudo(scope, node, &parsed.selectors) {
        Ok(Some(is_match)) => {
            rv.set_bool(is_match);
            return;
        }
        Ok(None) => {}
        Err(error) => {
            throw_native_selector_error_for_selector(scope, &parsed.selectors, &error);
            return;
        }
    }
    if let Some(result) = detached_native_matches_selector(scope, node, &parsed.selectors) {
        match result {
            Ok(is_match) => rv.set_bool(is_match),
            Err(error) => {
                throw_native_selector_error_for_selector(scope, &parsed.selectors, &error)
            }
        }
        return;
    }
    match detached_stylo_matches_selector(scope, node, &parsed.selectors) {
        Ok(is_match) => rv.set_bool(is_match),
        Err(error) => throw_native_selector_error_for_selector(scope, &parsed.selectors, &error),
    }
}

pub(in crate::native_bridge) fn bridge_detached_get_elements_by_tag_name_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentGetElementsByTagNameArgs>(scope, &args)
    else {
        return;
    };
    let matches =
        detached_native_element_query_objects(scope, root, |dom_host, root, include_root| {
            dom_host.elements_by_tag_name(root, &parsed.qualified_name, include_root)
        })
        .unwrap_or_else(|| {
            collect_detached_elements(scope, root, |scope, node| {
                detached_element_matches_tag_name(scope, node, &parsed.qualified_name)
            })
        });
    match build_detached_html_collection(scope, &matches) {
        Some(collection) => rv.set(collection.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_get_elements_by_tag_name_ns_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) =
        webidl::parse_args::<DetachedDocumentGetElementsByTagNameNsArgs>(scope, &args)
    else {
        return;
    };
    let namespace = normalize_namespace(parsed.namespace);
    let matches =
        detached_native_element_query_objects(scope, root, |dom_host, root, include_root| {
            dom_host.elements_by_tag_name_ns(
                root,
                namespace.as_deref(),
                &parsed.local_name,
                include_root,
            )
        })
        .unwrap_or_else(|| {
            collect_detached_elements(scope, root, |scope, node| {
                detached_element_matches_tag_name_ns(
                    scope,
                    node,
                    namespace.as_deref(),
                    &parsed.local_name,
                )
            })
        });
    match build_detached_html_collection(scope, &matches) {
        Some(collection) => rv.set(collection.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_get_elements_by_class_name_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) =
        webidl::parse_args::<DetachedDocumentGetElementsByClassNameArgs>(scope, &args)
    else {
        return;
    };
    if let Some(context) = root.get_creation_context(scope) {
        let scope = &mut v8::ContextScope::new(scope, context);
        if let Ok((runtime_ptr, handle)) =
            node_runtime_and_handle_from_object_or_detached(scope, root)
        {
            let include_root = node_is_document(unsafe { &*runtime_ptr }, handle);
            let collection = collections::build_live_collection_for_node(
                scope,
                runtime_ptr,
                handle,
                CollectionKind::HtmlCollection,
                LiveCollectionQueryKind::ClassName,
                Some(parsed.class_names),
                include_root,
            );
            rv.set(collection.into());
            return;
        }
    }
    let matches =
        detached_native_element_query_objects(scope, root, |dom_host, root, include_root| {
            dom_host.elements_by_class_name(root, &parsed.class_names, include_root)
        })
        .unwrap_or_else(|| {
            let insensitive =
                V8DetachedSelectorHost { scope }.quirks_mode(root) == QuirksMode::Quirks;
            let wanted: Vec<&str> = parsed.class_names.split_ascii_whitespace().collect();
            collect_detached_elements(scope, root, |scope, node| {
                if detached_selector_node_type(scope, node) == Some(1) {
                    let class_attr =
                        detached_element_attribute_value(scope, node, "class").unwrap_or_default();
                    let present: Vec<&str> = class_attr.split_ascii_whitespace().collect();
                    if !wanted.is_empty()
                        && wanted.iter().all(|name| {
                            present.iter().any(|actual| {
                                if insensitive {
                                    actual.eq_ignore_ascii_case(name)
                                } else {
                                    actual == name
                                }
                            })
                        })
                    {
                        return true;
                    }
                }
                false
            })
        });
    match build_detached_html_collection(scope, &matches) {
        Some(collection) => rv.set(collection.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_get_elements_by_name_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentGetElementsByNameArgs>(scope, &args)
    else {
        return;
    };
    let matches =
        detached_native_element_query_objects(scope, root, |dom_host, root, include_root| {
            dom_host.elements_by_name(root, &parsed.element_name, include_root)
        })
        .unwrap_or_else(|| {
            collect_detached_elements(scope, root, |scope, node| {
                !parsed.element_name.is_empty()
                    && detached_element_namespace_value(scope, node).as_deref() == Some(XHTML_NS)
                    && detached_element_attribute_value(scope, node, "name").as_deref()
                        == Some(parsed.element_name.as_str())
            })
        });
    match build_detached_html_collection(scope, &matches) {
        Some(collection) => rv.set(collection.into()),
        None => rv.set_null(),
    }
}

fn detached_element_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, node)
    {
        return unsafe { &*runtime_ptr }
            .dom_host()
            .get_attribute(handle, name);
    }
    let name = v8_string(scope, name)?;
    call_object_method(scope, node, "getAttribute", &[name.into()])
        .filter(|value| !value.is_null_or_undefined())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn detached_element_local_name_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    detached_element_local_name(scope, node)
        .or_else(|| object_string_property(scope, node, "localName"))
}

fn detached_element_namespace_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    detached_element_namespace_uri(scope, node)
        .or_else(|| object_string_property(scope, node, "namespaceURI"))
}

fn detached_selector_node_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<i32> {
    detached_node_type(scope, node)
}

fn detached_selector_node_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if detached_is_node(scope, node) {
        return match detached_node_type(scope, node) {
            Some(3 | 4 | 8) => Some(detached_character_data_value(scope, node)),
            _ => Some(String::new()),
        };
    }
    object_string_property(scope, node, "nodeValue")
}
