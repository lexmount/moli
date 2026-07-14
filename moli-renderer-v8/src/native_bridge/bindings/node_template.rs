mod character_data;
mod document_accessors;
mod document_methods;
mod element_accessors;
mod element_methods;
mod node_accessors;
mod node_methods;

use crate::{
    context_bootstrap::bridge_descriptor::BridgeDescriptor,
    native_bridge::{element, named_access},
};

pub(super) fn build_node_wrapper_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    descriptor: &BridgeDescriptor,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let _ = template.set_internal_field_count(1);

    if descriptor.prototype_name == "HTMLDocument" {
        named_access::install_document_named_property_handler(template);
    }

    node_accessors::install_node_accessors(scope, template, descriptor);
    document_accessors::install_document_accessors(scope, template, descriptor);
    element_accessors::install_element_accessors(scope, template, descriptor);
    node_methods::install_tree_mutation_and_relationship_methods(scope, template, descriptor);
    element_methods::install_html_element_action_methods(scope, template, descriptor);
    node_methods::install_node_utility_methods(scope, template);
    element_methods::install_element_query_and_attribute_methods(scope, template);
    document_methods::install_document_factory_methods(scope, template, descriptor);
    document_methods::install_document_lifecycle_and_query_methods(scope, template, descriptor);
    element_methods::install_extended_element_methods(scope, template, descriptor);
    element::install_specialized_template(
        scope,
        template,
        descriptor.specialized_template_installer,
    );

    template
}
