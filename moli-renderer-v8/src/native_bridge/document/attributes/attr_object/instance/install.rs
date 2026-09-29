use super::accessors::{
    attr_instance_base_uri_getter, attr_instance_local_name_getter, attr_instance_name_getter,
    attr_instance_namespace_uri_getter, attr_instance_node_name_getter,
    attr_instance_node_type_getter, attr_instance_owner_document_getter,
    attr_instance_owner_element_getter, attr_instance_prefix_getter,
    attr_instance_specified_getter, attr_instance_value_getter, attr_instance_value_setter,
};
use crate::definitions::{
    define_native_data_property as define_attr_instance_native_data_property,
    define_native_data_property_with_setter as define_attr_instance_native_data_property_with_setter,
};

pub(super) fn install_attr_instance_properties<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    define_attr_instance_native_data_property(
        scope,
        object,
        "nodeType",
        attr_instance_node_type_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "nodeName",
        attr_instance_node_name_getter,
    );
    define_attr_instance_native_data_property(scope, object, "name", attr_instance_name_getter);
    define_attr_instance_native_data_property(
        scope,
        object,
        "localName",
        attr_instance_local_name_getter,
    );
    define_attr_instance_native_data_property(scope, object, "prefix", attr_instance_prefix_getter);
    define_attr_instance_native_data_property(
        scope,
        object,
        "namespaceURI",
        attr_instance_namespace_uri_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "ownerElement",
        attr_instance_owner_element_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "ownerDocument",
        attr_instance_owner_document_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "specified",
        attr_instance_specified_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "baseURI",
        attr_instance_base_uri_getter,
    );
    define_attr_instance_native_data_property_with_setter(
        scope,
        object,
        "value",
        attr_instance_value_getter,
        attr_instance_value_setter,
    );
    define_attr_instance_native_data_property_with_setter(
        scope,
        object,
        "nodeValue",
        attr_instance_value_getter,
        attr_instance_value_setter,
    );
    define_attr_instance_native_data_property_with_setter(
        scope,
        object,
        "textContent",
        attr_instance_value_getter,
        attr_instance_value_setter,
    );
}
