use super::boundaries::{
    range_clone_range_callback, range_collapse_callback, range_select_node_callback,
    range_select_node_contents_callback, range_set_end_after_callback,
    range_set_end_before_callback, range_set_end_callback, range_set_start_after_callback,
    range_set_start_before_callback, range_set_start_callback,
};
use super::comparison::{
    range_compare_boundary_points_callback, range_compare_point_callback,
    range_intersects_node_callback, range_is_point_in_range_callback,
};
use super::construction::{document_create_range_callback, range_detach_callback};
use super::content::{
    range_clone_contents_callback, range_create_contextual_fragment_callback,
    range_delete_contents_callback, range_extract_contents_callback, range_insert_node_callback,
    range_surround_contents_callback, range_to_string_callback,
};
use super::geometry::{range_get_bounding_client_rect_callback, range_get_client_rects_callback};
use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Document, enumerable)]
struct DocumentRangePrototypeDeclaration {
    #[webapi(method, length = 0, callback = document_create_range_callback)]
    create_range: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Range, enumerable)]
struct RangePrototypeMethodsDeclaration {
    #[webapi(method, length = 2, callback = range_set_start_callback)]
    set_start: (),
    #[webapi(method, length = 2, callback = range_set_end_callback)]
    set_end: (),
    #[webapi(method, length = 1, callback = range_select_node_contents_callback)]
    select_node_contents: (),
    #[webapi(method, length = 0, callback = range_clone_contents_callback)]
    clone_contents: (),
    #[webapi(method, length = 0, callback = range_collapse_callback)]
    collapse: (),
    #[webapi(method, length = 1, callback = range_select_node_callback)]
    select_node: (),
    #[webapi(method, length = 1, callback = range_set_start_before_callback)]
    set_start_before: (),
    #[webapi(method, length = 1, callback = range_set_start_after_callback)]
    set_start_after: (),
    #[webapi(method, length = 1, callback = range_set_end_before_callback)]
    set_end_before: (),
    #[webapi(method, length = 1, callback = range_set_end_after_callback)]
    set_end_after: (),
    #[webapi(method, length = 0, callback = range_clone_range_callback)]
    clone_range: (),
    #[webapi(method, length = 0, callback = range_to_string_callback)]
    to_string: (),
    #[webapi(method, length = 2, callback = range_compare_point_callback)]
    compare_point: (),
    #[webapi(method, length = 2, callback = range_is_point_in_range_callback)]
    is_point_in_range: (),
    #[webapi(method, length = 1, callback = range_intersects_node_callback)]
    intersects_node: (),
    #[webapi(method, length = 2, callback = range_compare_boundary_points_callback)]
    compare_boundary_points: (),
    #[webapi(method, length = 1, callback = range_insert_node_callback)]
    insert_node: (),
    #[webapi(method, length = 1, callback = range_create_contextual_fragment_callback)]
    create_contextual_fragment: (),
    #[webapi(method, length = 0, callback = range_delete_contents_callback)]
    delete_contents: (),
    #[webapi(method, length = 0, callback = range_extract_contents_callback)]
    extract_contents: (),
    #[webapi(method, length = 1, callback = range_surround_contents_callback)]
    surround_contents: (),
    #[webapi(method, length = 0, callback = range_get_bounding_client_rect_callback)]
    get_bounding_client_rect: (),
    #[webapi(method, length = 0, callback = range_get_client_rects_callback)]
    get_client_rects: (),
    #[webapi(method, length = 0, callback = range_detach_callback)]
    detach: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AbstractRange)]
struct AbstractRangePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = super::accessors::range_attribute_getter_callback,
        data = crate::util::callback_data_index_value(scope, 0),
        enumerable
    )]
    start_container: (),
    #[webapi(
        accessor_property,
        getter = super::accessors::range_attribute_getter_callback,
        data = crate::util::callback_data_index_value(scope, 1),
        enumerable
    )]
    start_offset: (),
    #[webapi(
        accessor_property,
        getter = super::accessors::range_attribute_getter_callback,
        data = crate::util::callback_data_index_value(scope, 2),
        enumerable
    )]
    end_container: (),
    #[webapi(
        accessor_property,
        getter = super::accessors::range_attribute_getter_callback,
        data = crate::util::callback_data_index_value(scope, 3),
        enumerable
    )]
    end_offset: (),
    #[webapi(
        accessor_property,
        getter = super::accessors::range_attribute_getter_callback,
        data = crate::util::callback_data_index_value(scope, 4),
        enumerable
    )]
    collapsed: (),
    #[webapi(
        accessor_property,
        getter = super::accessors::range_attribute_getter_callback,
        data = crate::util::callback_data_index_value(scope, 5),
        enumerable
    )]
    common_ancestor_container: (),
}

pub(in crate::context_bootstrap) fn reset_range_runtime_state(scope: &mut v8::PinScope<'_, '_>) {
    clear_live_range_registry(scope);
}

pub(in crate::context_bootstrap) fn install_range_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Document" => {
            DocumentRangePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "Range" => {
            RangePrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "AbstractRange" => {
            AbstractRangePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}
