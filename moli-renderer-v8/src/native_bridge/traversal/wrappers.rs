use crate::web_api_interfaces;
use std::ffi::c_void;

use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, NativeDomBridge},
};
use moli_webapi_declare::WebApiFunctionTemplate;

use super::filters::TraversalFilter;
use super::{
    node_iterator_detach_callback, node_iterator_filter_getter, node_iterator_next_node_callback,
    node_iterator_pointer_before_reference_node_getter, node_iterator_previous_node_callback,
    node_iterator_reference_node_getter, node_iterator_root_getter,
    node_iterator_what_to_show_getter, tree_walker_current_node_getter,
    tree_walker_current_node_setter, tree_walker_filter_getter, tree_walker_first_child_callback,
    tree_walker_last_child_callback, tree_walker_next_node_callback,
    tree_walker_next_sibling_callback, tree_walker_parent_node_callback,
    tree_walker_previous_node_callback, tree_walker_previous_sibling_callback,
    tree_walker_root_getter, tree_walker_what_to_show_getter,
};
use crate::native_bridge::bindings::set_named_constructor_prototype;
use crate::webidl::WebIdlCallbackInterface;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NodeIterator, enumerable)]
struct NodeIteratorPrototypeDeclaration {
    #[webapi(accessor_property, getter = node_iterator_root_getter)]
    root: (),
    #[webapi(accessor_property, getter = node_iterator_what_to_show_getter)]
    what_to_show: (),
    #[webapi(accessor_property, getter = node_iterator_filter_getter)]
    filter: (),
    #[webapi(accessor_property, getter = node_iterator_reference_node_getter)]
    reference_node: (),
    #[webapi(accessor_property, getter = node_iterator_pointer_before_reference_node_getter)]
    pointer_before_reference_node: (),
    #[webapi(method, length = 0, callback = node_iterator_next_node_callback)]
    next_node: (),
    #[webapi(method, length = 0, callback = node_iterator_previous_node_callback)]
    previous_node: (),
    #[webapi(method, length = 0, callback = node_iterator_detach_callback)]
    detach: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TreeWalker, enumerable)]
struct TreeWalkerPrototypeDeclaration {
    #[webapi(accessor_property, getter = tree_walker_root_getter)]
    root: (),
    #[webapi(accessor_property, getter = tree_walker_what_to_show_getter)]
    what_to_show: (),
    #[webapi(accessor_property, getter = tree_walker_filter_getter)]
    filter: (),
    #[webapi(
        accessor_property,
        getter = tree_walker_current_node_getter,
        setter = tree_walker_current_node_setter
    )]
    current_node: (),
    #[webapi(method, length = 0, callback = tree_walker_parent_node_callback)]
    parent_node: (),
    #[webapi(method, length = 0, callback = tree_walker_first_child_callback)]
    first_child: (),
    #[webapi(method, length = 0, callback = tree_walker_last_child_callback)]
    last_child: (),
    #[webapi(method, length = 0, callback = tree_walker_next_sibling_callback)]
    next_sibling: (),
    #[webapi(method, length = 0, callback = tree_walker_previous_sibling_callback)]
    previous_sibling: (),
    #[webapi(method, length = 0, callback = tree_walker_next_node_callback)]
    next_node: (),
    #[webapi(method, length = 0, callback = tree_walker_previous_node_callback)]
    previous_node: (),
}

pub(in crate::native_bridge) fn build_node_iterator_wrapper_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let _ = template.set_internal_field_count(2);
    template
}

pub(in crate::native_bridge) fn build_tree_walker_wrapper_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let _ = template.set_internal_field_count(2);
    template
}

pub(crate) fn install_traversal_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "NodeIterator" => {
            NodeIteratorPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "TreeWalker" => {
            TreeWalkerPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(in crate::native_bridge) fn build_node_iterator_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    bridge: &mut NativeDomBridge,
    root: DomHandle,
    what_to_show: u32,
    filter: Option<WebIdlCallbackInterface>,
) -> v8::Local<'s, v8::Object> {
    let filter = filter.map(|filter| TraversalFilter::new(scope, runtime_ptr, filter));
    let id = bridge.register_node_iterator(root, what_to_show, filter);
    let template = bridge.node_iterator_wrapper_template();
    let wrapper = template
        .new_instance(scope)
        .expect("failed to instantiate NodeIterator wrapper");
    let runtime_external = v8::External::new(scope, runtime_ptr as *mut c_void);
    assert!(
        wrapper.set_internal_field(0, runtime_external.into()),
        "NodeIterator wrapper must expose its runtime field"
    );
    assert!(
        wrapper.set_internal_field(1, v8::Number::new(scope, id as f64).into()),
        "NodeIterator wrapper must expose its state id field"
    );
    set_named_constructor_prototype(scope, wrapper, "NodeIterator");
    wrapper
}

pub(in crate::native_bridge) fn build_tree_walker_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    bridge: &mut NativeDomBridge,
    root: DomHandle,
    what_to_show: u32,
    filter: Option<WebIdlCallbackInterface>,
) -> v8::Local<'s, v8::Object> {
    let filter = filter.map(|filter| TraversalFilter::new(scope, runtime_ptr, filter));
    let id = bridge.register_tree_walker(root, what_to_show, filter);
    let template = bridge.tree_walker_wrapper_template();
    let wrapper = template
        .new_instance(scope)
        .expect("failed to instantiate TreeWalker wrapper");
    let runtime_external = v8::External::new(scope, runtime_ptr as *mut c_void);
    assert!(
        wrapper.set_internal_field(0, runtime_external.into()),
        "TreeWalker wrapper must expose its runtime field"
    );
    assert!(
        wrapper.set_internal_field(1, v8::Number::new(scope, id as f64).into()),
        "TreeWalker wrapper must expose its state id field"
    );
    set_named_constructor_prototype(scope, wrapper, "TreeWalker");
    wrapper
}
