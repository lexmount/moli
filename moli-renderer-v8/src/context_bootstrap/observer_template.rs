use super::media_queries::{
    media_query_list_add_event_listener_callback, media_query_list_add_listener_callback,
    media_query_list_dispatch_event_callback, media_query_list_remove_event_listener_callback,
    media_query_list_remove_listener_callback,
};
use super::performance_runtime::{
    performance_entry_list_get_entries_by_name_callback,
    performance_entry_list_get_entries_by_type_callback,
    performance_entry_list_get_entries_callback, performance_observer_disconnect_callback,
    performance_observer_observe_callback, performance_observer_take_records_callback,
};
use super::resize_observer_runtime::{
    resize_observer_disconnect_callback, resize_observer_observe_callback,
    resize_observer_take_records_callback, resize_observer_unobserve_callback,
};
use crate::observer_runtime;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MutationObserver, enumerable)]
struct MutationObserverTemplateMethodsDeclaration {
    #[webapi(method, length = 2, callback = observer_runtime::mutation_observer_observe_callback)]
    observe: (),

    #[webapi(
        method,
        length = 0,
        callback = observer_runtime::mutation_observer_disconnect_callback
    )]
    disconnect: (),

    #[webapi(
        method,
        length = 0,
        callback = observer_runtime::mutation_observer_take_records_callback
    )]
    take_records: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IntersectionObserver, enumerable)]
struct IntersectionObserverTemplateMethodsDeclaration {
    #[webapi(
        method,
        length = 1,
        callback = observer_runtime::intersection_observer_observe_callback
    )]
    observe: (),

    #[webapi(
        method,
        length = 1,
        callback = observer_runtime::intersection_observer_unobserve_callback
    )]
    unobserve: (),

    #[webapi(
        method,
        length = 0,
        callback = observer_runtime::intersection_observer_disconnect_callback
    )]
    disconnect: (),

    #[webapi(
        method,
        length = 0,
        callback = observer_runtime::intersection_observer_take_records_callback
    )]
    take_records: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaQueryList, enumerable)]
struct MediaQueryListTemplateMethodsDeclaration {
    #[webapi(method, length = 2, callback = media_query_list_add_event_listener_callback)]
    add_event_listener: (),

    #[webapi(
        method,
        length = 2,
        callback = media_query_list_remove_event_listener_callback
    )]
    remove_event_listener: (),

    #[webapi(method, length = 1, callback = media_query_list_dispatch_event_callback)]
    dispatch_event: (),

    #[webapi(method, length = 1, callback = media_query_list_add_listener_callback)]
    add_listener: (),

    #[webapi(method, length = 1, callback = media_query_list_remove_listener_callback)]
    remove_listener: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ResizeObserver, enumerable)]
struct ResizeObserverTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = resize_observer_observe_callback)]
    observe: (),

    #[webapi(method, length = 1, callback = resize_observer_unobserve_callback)]
    unobserve: (),

    #[webapi(method, length = 0, callback = resize_observer_disconnect_callback)]
    disconnect: (),

    #[webapi(method, length = 0, callback = resize_observer_take_records_callback)]
    take_records: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceObserver, enumerable)]
struct PerformanceObserverTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = performance_observer_observe_callback)]
    observe: (),

    #[webapi(method, length = 0, callback = performance_observer_disconnect_callback)]
    disconnect: (),

    #[webapi(method, length = 0, callback = performance_observer_take_records_callback)]
    take_records: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceObserverEntryList, enumerable)]
struct PerformanceObserverEntryListTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = performance_entry_list_get_entries_callback)]
    get_entries: (),

    #[webapi(
        method,
        length = 1,
        callback = performance_entry_list_get_entries_by_type_callback
    )]
    get_entries_by_type: (),

    #[webapi(
        method,
        length = 2,
        callback = performance_entry_list_get_entries_by_name_callback
    )]
    get_entries_by_name: (),
}

pub(super) fn install_observer_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    spec_name: &str,
) {
    match spec_name {
        "MutationObserver" => {
            let proto = template.prototype_template(scope);
            MutationObserverTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "IntersectionObserver" => {
            let proto = template.prototype_template(scope);
            IntersectionObserverTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "MediaQueryList" => {
            let proto = template.prototype_template(scope);
            MediaQueryListTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "ResizeObserver" => {
            let proto = template.prototype_template(scope);
            ResizeObserverTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "PerformanceObserver" => {
            let proto = template.prototype_template(scope);
            PerformanceObserverTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "PerformanceObserverEntryList" => {
            let proto = template.prototype_template(scope);
            PerformanceObserverEntryListTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        _ => {}
    }
}
