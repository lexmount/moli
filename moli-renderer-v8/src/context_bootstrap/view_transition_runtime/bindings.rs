use super::{
    document::{document_active_view_transition_getter, document_start_view_transition_callback},
    lifecycle::{
        view_transition_finished_getter, view_transition_ready_getter,
        view_transition_skip_callback, view_transition_types_getter,
        view_transition_update_callback_done_getter, view_transition_wait_until_callback,
    },
    type_set::{
        view_transition_type_set_add_callback, view_transition_type_set_clear_callback,
        view_transition_type_set_delete_callback, view_transition_type_set_entries_callback,
        view_transition_type_set_for_each_callback, view_transition_type_set_has_callback,
        view_transition_type_set_keys_callback, view_transition_type_set_size_getter,
        view_transition_type_set_values_callback,
    },
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Document, enumerable)]
struct DocumentViewTransitionTemplateDeclaration {
    #[webapi(
        method = "startViewTransition",
        length = 0,
        callback = document_start_view_transition_callback
    )]
    start_view_transition: (),

    #[webapi(
        accessor_property = "activeViewTransition",
        getter = document_active_view_transition_getter
    )]
    active_view_transition: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ViewTransition, enumerable)]
struct ViewTransitionTemplateDeclaration {
    #[webapi(method = "skipTransition", length = 0, callback = view_transition_skip_callback)]
    skip_transition: (),

    #[webapi(method = "waitUntil", length = 1, callback = view_transition_wait_until_callback)]
    wait_until: (),

    #[webapi(accessor_property, getter = view_transition_ready_getter)]
    ready: (),

    #[webapi(accessor_property, getter = view_transition_finished_getter)]
    finished: (),

    #[webapi(
        accessor_property = "updateCallbackDone",
        getter = view_transition_update_callback_done_getter
    )]
    update_callback_done: (),

    #[webapi(accessor_property, getter = view_transition_types_getter)]
    types: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ViewTransitionTypeSet, enumerable)]
struct ViewTransitionTypeSetTemplateDeclaration {
    #[webapi(accessor_property, getter = view_transition_type_set_size_getter)]
    size: (),

    #[webapi(method, length = 1, callback = view_transition_type_set_add_callback)]
    add: (),

    #[webapi(method, length = 0, callback = view_transition_type_set_clear_callback)]
    clear: (),

    #[webapi(method, length = 1, callback = view_transition_type_set_delete_callback)]
    delete: (),

    #[webapi(method, length = 1, callback = view_transition_type_set_has_callback)]
    has: (),

    #[webapi(method, length = 0, callback = view_transition_type_set_entries_callback)]
    entries: (),

    #[webapi(
        method = "forEach",
        length = 1,
        callback = view_transition_type_set_for_each_callback
    )]
    for_each: (),

    #[webapi(method, length = 0, callback = view_transition_type_set_keys_callback)]
    keys: (),

    #[webapi(method, length = 0, callback = view_transition_type_set_values_callback)]
    values: (),

    #[webapi(alias = "values", symbol = "iterator")]
    iterator: (),
}

pub(in crate::context_bootstrap) fn install_view_transition_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Document" => {
            DocumentViewTransitionTemplateDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "ViewTransition" => {
            ViewTransitionTemplateDeclaration::initialize_prototype_template(scope, prototype);
        }
        "ViewTransitionTypeSet" => {
            ViewTransitionTypeSetTemplateDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}
