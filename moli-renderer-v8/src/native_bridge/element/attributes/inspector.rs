use crate::custom_elements;
use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::native_bridge::document::clear_live_attr_cache_entry_ns;
use crate::native_bridge::{JsContextHost, validate_attribute_name};
use crate::runtime::{RendererDomAttributeMutation, RendererDomAttributeMutationOutcome};

use super::super::{
    remove_live_element_attribute_appending_to_current_reaction_queue,
    set_live_element_attribute_appending_to_current_reaction_queue,
    update_iframe_snapshot_navigation,
};
use super::mutation::attribute_target_for_remove_name;

fn attribute_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    normalized_name: &str,
) -> Option<String> {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .and_then(|element| element.attribute(normalized_name))
        .map(str::to_owned)
}

pub(crate) fn mutate_live_element_attribute_for_inspector(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    mutation: RendererDomAttributeMutation,
) -> RendererDomAttributeMutationOutcome {
    let runtime = unsafe { &*runtime_ptr };
    let Some(node) = runtime.dom_host().node(handle) else {
        return RendererDomAttributeMutationOutcome::NodeNotFound;
    };
    let Some(element) = node.as_element() else {
        return RendererDomAttributeMutationOutcome::NodeNotElement;
    };

    match mutation {
        RendererDomAttributeMutation::Set { name, value } => {
            if !validate_attribute_name(&name) {
                return RendererDomAttributeMutationOutcome::InvalidName { name };
            }
            let normalized_name = runtime
                .dom_host()
                .normalized_attribute_name(handle, &name)
                .unwrap_or_else(|| name.clone());
            let old_value = element.attribute(&normalized_name).map(str::to_owned);
            let updates_iframe_navigation =
                normalized_name == "src" && element.is_html_element("iframe");

            let new_value = if updates_iframe_navigation {
                update_iframe_snapshot_navigation(scope, runtime_ptr, handle, &value);
                attribute_value(unsafe { &*runtime_ptr }, handle, &normalized_name)
            } else {
                let mut new_value = old_value.clone();
                custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
                    let _ = set_live_element_attribute_appending_to_current_reaction_queue(
                        scope,
                        runtime_ptr,
                        handle,
                        &name,
                        &value,
                    );
                    new_value = attribute_value(unsafe { &*runtime_ptr }, handle, &normalized_name);
                });
                new_value
            };

            RendererDomAttributeMutationOutcome::Applied {
                new_value,
                name: normalized_name,
                old_value,
            }
        }
        RendererDomAttributeMutation::Remove { name } => {
            let normalized_name = runtime
                .dom_host()
                .normalized_attribute_name(handle, &name)
                .unwrap_or_else(|| name.clone());
            let old_value = element.attribute(&normalized_name).map(str::to_owned);
            let attr_cache_target =
                attribute_target_for_remove_name(unsafe { &*runtime_ptr }, handle, &name);

            if let Some((namespace, local_name)) = attr_cache_target
                && let Some(wrapper) = unsafe { &mut *runtime_ptr }
                    .native_bridge_mut()
                    .cached_handle_wrapper(scope, handle)
            {
                clear_live_attr_cache_entry_ns(scope, wrapper, namespace.as_deref(), &local_name);
            }
            let mut new_value = old_value.clone();
            custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
                let _ = remove_live_element_attribute_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    handle,
                    &name,
                );
                new_value = attribute_value(unsafe { &*runtime_ptr }, handle, &normalized_name);
            });

            RendererDomAttributeMutationOutcome::Applied {
                new_value,
                name: normalized_name,
                old_value,
            }
        }
    }
}
