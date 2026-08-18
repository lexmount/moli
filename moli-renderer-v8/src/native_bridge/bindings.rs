use std::{collections::HashMap, ffi::c_void};

use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

use super::super::context_bootstrap::bridge_descriptor::{
    WrapperKind, node_bridge_descriptor, node_bridge_descriptors,
};
use super::super::reflector::ReflectorId;
use super::{BridgeHandle, JsContextHost, collections, document, element, traversal, window};

mod native_template;
mod node_template;

use native_template::build_native_bridge_template;
use node_template::build_node_wrapper_template;

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct NativeBridgeGlobalDeclaration<'scope> {
    #[webapi(data_property = "__moliNativeBridge")]
    bridge: v8::Local<'scope, v8::Object>,
}

#[cfg(test)]
thread_local! {
    static WRAPPER_OWNER_REALM_CUSTOM_ELEMENT_CHECKS: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_wrapper_owner_realm_custom_element_checks_for_test() {
    WRAPPER_OWNER_REALM_CUSTOM_ELEMENT_CHECKS.set(0);
}

#[cfg(test)]
pub(crate) fn wrapper_owner_realm_custom_element_checks_for_test() -> u64 {
    WRAPPER_OWNER_REALM_CUSTOM_ELEMENT_CHECKS.get()
}

#[cfg(test)]
fn record_wrapper_owner_realm_custom_element_check_for_test() {
    WRAPPER_OWNER_REALM_CUSTOM_ELEMENT_CHECKS.set(
        WRAPPER_OWNER_REALM_CUSTOM_ELEMENT_CHECKS
            .get()
            .saturating_add(1),
    );
}

pub(crate) struct NativeBridgeBindings {
    isolate_ptr: v8::UnsafeRawIsolatePtr,
    window_global_template: v8::Global<v8::ObjectTemplate>,
    cross_origin_window_global_template: v8::Global<v8::ObjectTemplate>,
    bridge_template: v8::Global<v8::ObjectTemplate>,
    node_wrapper_templates: HashMap<&'static str, v8::Global<v8::ObjectTemplate>>,
    window_wrapper_template: v8::Global<v8::ObjectTemplate>,
    collection_wrapper_template: v8::Global<v8::ObjectTemplate>,
    static_handle_node_list_wrapper_template: v8::Global<v8::ObjectTemplate>,
    live_collection_wrapper_template: v8::Global<v8::ObjectTemplate>,
    node_iterator_wrapper_template: v8::Global<v8::ObjectTemplate>,
    tree_walker_wrapper_template: v8::Global<v8::ObjectTemplate>,
    dom_token_list_wrapper_template: v8::Global<v8::ObjectTemplate>,
    dom_string_map_wrapper_template: v8::Global<v8::ObjectTemplate>,
    style_wrapper_template: v8::Global<v8::ObjectTemplate>,
    named_node_map_wrapper_template: v8::Global<v8::ObjectTemplate>,
}

fn wrapper_kind_for_handle(handle: &BridgeHandle) -> WrapperKind {
    match handle {
        BridgeHandle::Window => WrapperKind::Window,
        BridgeHandle::ClassList(_, _) => WrapperKind::ClassList,
        BridgeHandle::Dataset(_) => WrapperKind::Dataset,
        BridgeHandle::Style(_) => WrapperKind::Style,
        BridgeHandle::ComputedStyle(_, _) => WrapperKind::ComputedStyle,
        BridgeHandle::Node(_) => WrapperKind::Node,
    }
}

fn prototype_name_for_handle(host_ptr: *mut JsContextHost, handle: &BridgeHandle) -> &'static str {
    match handle {
        BridgeHandle::Window => "Window",
        BridgeHandle::ClassList(_, _) => "DOMTokenList",
        BridgeHandle::Dataset(_) => "DOMStringMap",
        BridgeHandle::Style(_) | BridgeHandle::ComputedStyle(_, _) => "CSSStyleProperties",
        BridgeHandle::Node(node_handle) => {
            let runtime = unsafe { &*host_ptr };
            if runtime.dom_host().is_shadow_root(*node_handle) {
                "ShadowRoot"
            } else {
                runtime
                    .dom_host()
                    .node(*node_handle)
                    .map(|node| match node.data() {
                        crate::dom::native::NodeData::Document(document) => {
                            if document.is_html_document() {
                                "HTMLDocument"
                            } else {
                                "XMLDocument"
                            }
                        }
                        _ if node
                            .local_name()
                            .is_some_and(crate::custom_elements::is_valid_custom_element_name)
                            && runtime
                                .custom_elements_for_node_handle(*node_handle)
                                .is_some_and(|store| {
                                    store.is_failed_construction_handle(*node_handle)
                                }) =>
                        {
                            "HTMLUnknownElement"
                        }
                        _ => node.wrapper_prototype_name(),
                    })
                    .unwrap_or("Node")
            }
        }
    }
}

impl NativeBridgeBindings {
    pub(crate) fn build(
        scope: &mut v8::PinScope<'_, '_, ()>,
        isolate_ptr: v8::UnsafeRawIsolatePtr,
        window_global_template: v8::Local<'_, v8::ObjectTemplate>,
        cross_origin_window_global_template: v8::Local<'_, v8::ObjectTemplate>,
    ) -> Self {
        let mut node_wrapper_templates = HashMap::new();
        for descriptor in node_bridge_descriptors() {
            let template = build_node_wrapper_template(scope, descriptor);
            node_wrapper_templates
                .insert(descriptor.prototype_name, v8::Global::new(scope, template));
        }

        let bridge_template = build_native_bridge_template(scope);
        let window_wrapper_template = window::build_window_wrapper_template(scope);
        let collection_wrapper_template = collections::build_collection_wrapper_template(scope);
        let static_handle_node_list_wrapper_template =
            collections::build_static_handle_node_list_wrapper_template(scope);
        let live_collection_wrapper_template =
            collections::build_live_collection_wrapper_template(scope);
        let node_iterator_wrapper_template = traversal::build_node_iterator_wrapper_template(scope);
        let tree_walker_wrapper_template = traversal::build_tree_walker_wrapper_template(scope);
        let dom_token_list_wrapper_template = element::build_dom_token_list_wrapper_template(scope);
        let dom_string_map_wrapper_template = element::build_dom_string_map_wrapper_template(scope);
        let style_wrapper_template = element::build_style_wrapper_template(scope);
        let named_node_map_wrapper_template =
            document::build_named_node_map_wrapper_template(scope);

        Self {
            isolate_ptr,
            window_global_template: v8::Global::new(scope, window_global_template),
            cross_origin_window_global_template: v8::Global::new(
                scope,
                cross_origin_window_global_template,
            ),
            bridge_template: v8::Global::new(scope, bridge_template),
            node_wrapper_templates,
            window_wrapper_template: v8::Global::new(scope, window_wrapper_template),
            collection_wrapper_template: v8::Global::new(scope, collection_wrapper_template),
            static_handle_node_list_wrapper_template: v8::Global::new(
                scope,
                static_handle_node_list_wrapper_template,
            ),
            live_collection_wrapper_template: v8::Global::new(
                scope,
                live_collection_wrapper_template,
            ),
            node_iterator_wrapper_template: v8::Global::new(scope, node_iterator_wrapper_template),
            tree_walker_wrapper_template: v8::Global::new(scope, tree_walker_wrapper_template),
            dom_token_list_wrapper_template: v8::Global::new(
                scope,
                dom_token_list_wrapper_template,
            ),
            dom_string_map_wrapper_template: v8::Global::new(
                scope,
                dom_string_map_wrapper_template,
            ),
            style_wrapper_template: v8::Global::new(scope, style_wrapper_template),
            named_node_map_wrapper_template: v8::Global::new(
                scope,
                named_node_map_wrapper_template,
            ),
        }
    }

    pub(crate) fn window_global_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::ObjectTemplate> {
        v8::Local::new(scope, &self.window_global_template)
    }

    /// Install the native bridge global object with two internal fields:
    /// - field 0: `*mut JsContextHost` (for `runtime_ptr_from_object` compatibility)
    /// - field 1: `*const RefCell<JsContextHost>` from the per-context bridge-ref token
    pub(super) fn install_global<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
        host_ptr: *mut JsContextHost,
        rc_ptr: *mut c_void,
    ) -> Result<()> {
        let template = self.bridge_template();
        let bridge = template
            .new_instance(scope)
            .ok_or_else(|| anyhow!("failed to create native bridge object"))?;
        let host_external = v8::External::new(scope, host_ptr as *mut c_void);
        let _ = bridge.set_internal_field(0, host_external.into());
        // Field 1 stores the Rc pointer owned by the context's bridge-ref token.
        let rc_external = v8::External::new(scope, rc_ptr);
        let _ = bridge.set_internal_field(1, rc_external.into());

        NativeBridgeGlobalDeclaration::new(bridge)
            .initialize(scope, global)
            .map_err(|error| anyhow!("failed to install native bridge global: {error}"))
    }

    pub(super) fn instantiate_wrapper<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
        handle: BridgeHandle,
        reflector_id: ReflectorId,
    ) -> v8::Local<'s, v8::Object> {
        let prototype_name = prototype_name_for_handle(host_ptr, &handle);
        let wrapper_kind = wrapper_kind_for_handle(&handle);
        let template = match wrapper_kind {
            WrapperKind::Window => self.window_wrapper_template(),
            WrapperKind::ClassList => self.dom_token_list_wrapper_template(),
            WrapperKind::Dataset => self.dom_string_map_wrapper_template(),
            WrapperKind::Style | WrapperKind::ComputedStyle => self.style_wrapper_template(),
            WrapperKind::Node => self
                .node_wrapper_template(prototype_name)
                .unwrap_or_else(|| {
                    panic!("missing native wrapper template for `{prototype_name}`")
                }),
        };

        let wrapper = template
            .new_instance(scope)
            .unwrap_or_else(|| panic!("failed to instantiate `{prototype_name}` wrapper"));
        let host_external = v8::External::new(scope, host_ptr as *mut c_void);
        assert!(
            wrapper.set_internal_field(0, host_external.into()),
            "`{prototype_name}` wrapper must expose its runtime field"
        );
        // Field 1 stores a reflector-backed identity id rather than the raw DomHandle.
        assert!(
            wrapper.set_internal_field(1, v8::Number::new(scope, reflector_id.raw() as f64).into()),
            "`{prototype_name}` wrapper must expose its reflector identity field"
        );
        set_named_constructor_prototype(scope, wrapper, prototype_name);
        self.sync_wrapper_owner_realm_prototype(scope, host_ptr, &handle, wrapper);
        if matches!(wrapper_kind, WrapperKind::Node) {
            let descriptor = node_bridge_descriptor(prototype_name).unwrap_or_else(|| {
                panic!("missing native bridge descriptor for `{prototype_name}`")
            });
            element::install_specialized_instance_properties(
                scope,
                wrapper,
                descriptor.runtime_install_groups,
            );
        }
        wrapper
    }

    pub(super) fn sync_wrapper_owner_realm_prototype(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: &BridgeHandle,
        wrapper: v8::Local<'_, v8::Object>,
    ) {
        if matches!(handle, BridgeHandle::Window) {
            window::sync_window_wrapper_function_identity(scope, wrapper);
            return;
        }
        let BridgeHandle::Node(node_handle) = handle else {
            return;
        };
        let child_handle = {
            let host = unsafe { &*host_ptr };
            let Some(document_handle) = host.dom_host().owner_document_handle(*node_handle) else {
                return;
            };
            let Some(child_handle) =
                host.child_browsing_context_host_for_document_handle(document_handle)
            else {
                return;
            };
            child_handle
        };
        #[cfg(test)]
        record_wrapper_owner_realm_custom_element_check_for_test();
        if unsafe { &*host_ptr }.custom_element_handle_is_upgraded(*node_handle) {
            return;
        }
        let prototype_name = prototype_name_for_handle(host_ptr, handle);
        if let Some(prototype) = unsafe { &mut *host_ptr }
            .child_browsing_context_constructor_prototype(scope, child_handle, prototype_name)
        {
            let updated = wrapper.set_prototype(scope, prototype).unwrap_or_else(|| {
                panic!("failed to set child-realm `{prototype_name}` wrapper prototype")
            });
            assert!(
                updated,
                "V8 rejected the child-realm `{prototype_name}` wrapper prototype"
            );
        }
    }

    pub(super) fn instantiate_window_shell<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
    ) -> v8::Local<'s, v8::Object> {
        let wrapper = self
            .window_wrapper_template()
            .new_instance(scope)
            .expect("failed to instantiate synthetic Window wrapper");
        let host_external = v8::External::new(scope, host_ptr as *mut c_void);
        assert!(
            wrapper.set_internal_field(0, host_external.into()),
            "synthetic Window wrapper must expose its runtime field"
        );
        assert!(
            wrapper.set_internal_field(1, v8::Number::new(scope, 0.0).into()),
            "synthetic Window wrapper must expose its identity field"
        );
        set_named_constructor_prototype(scope, wrapper, "Window");
        wrapper
    }

    pub(super) fn instantiate_window_proxy_shell<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
    ) -> (v8::Local<'s, v8::Object>, v8::Global<v8::Context>) {
        // A cross-origin facade exposes a deliberately restricted Window
        // surface. It must not inherit the same-origin [Global] own
        // properties, especially non-configurable Window.location, before
        // its cross-origin accessors are installed.
        let global_template = v8::Local::new(scope, &self.cross_origin_window_global_template);
        let parent_security_token = scope.get_current_context().get_security_token(scope);
        let context = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_template: Some(global_template),
                ..Default::default()
            },
        );
        context.set_security_token(parent_security_token);
        let global = context.global(scope);
        (global, v8::Global::new(scope, context))
    }

    pub(super) fn attach_window_proxy_shell_to_facade<'s, 'i>(
        &self,
        scope: &mut v8::PinScope<'s, 'i, ()>,
        window_proxy: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Context>> {
        let global_template = v8::Local::new(scope, &self.cross_origin_window_global_template);
        let context = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_template: Some(global_template),
                global_object: Some(window_proxy.into()),
                ..Default::default()
            },
        );
        context
            .global(scope)
            .strict_equals(window_proxy.into())
            .then_some(context)
    }

    pub(super) fn collection_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.collection_wrapper_template.open(isolate)
    }

    pub(super) fn static_handle_node_list_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.static_handle_node_list_wrapper_template.open(isolate)
    }

    pub(super) fn live_collection_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.live_collection_wrapper_template.open(isolate)
    }

    pub(super) fn node_iterator_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.node_iterator_wrapper_template.open(isolate)
    }

    pub(super) fn tree_walker_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.tree_walker_wrapper_template.open(isolate)
    }

    fn bridge_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.bridge_template.open(isolate)
    }

    fn node_wrapper_template(
        &mut self,
        prototype_name: &'static str,
    ) -> Option<&v8::ObjectTemplate> {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.node_wrapper_templates
            .get(prototype_name)
            .map(|template| template.open(isolate))
    }

    fn window_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.window_wrapper_template.open(isolate)
    }

    fn dom_token_list_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.dom_token_list_wrapper_template.open(isolate)
    }

    fn dom_string_map_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.dom_string_map_wrapper_template.open(isolate)
    }

    fn style_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.style_wrapper_template.open(isolate)
    }

    pub(super) fn named_node_map_wrapper_template(&mut self) -> &v8::ObjectTemplate {
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut self.isolate_ptr) };
        self.named_node_map_wrapper_template.open(isolate)
    }
}

pub(super) fn set_named_constructor_prototype(
    scope: &mut v8::PinScope<'_, '_>,
    wrapper: v8::Local<'_, v8::Object>,
    constructor_name: &str,
) {
    // Wrapper identity is tied to the realm's trusted interface, not the
    // author-visible global property. The latter is configurable and may have
    // been replaced after lazy interface materialization.
    let prototype =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, constructor_name)
            .unwrap_or_else(|error| {
                panic!("failed to materialize intrinsic `{constructor_name}` prototype: {error}")
            });
    let updated = wrapper
        .set_prototype(scope, prototype.into())
        .unwrap_or_else(|| panic!("failed to set `{constructor_name}` wrapper prototype"));
    assert!(
        updated,
        "V8 rejected the intrinsic `{constructor_name}` wrapper prototype"
    );
}
