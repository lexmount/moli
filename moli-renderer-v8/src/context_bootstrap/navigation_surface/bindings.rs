use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::History, enumerable)]
struct HistoryTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = history_back_callback)]
    back: (),

    #[webapi(method, length = 0, callback = history_forward_callback)]
    forward: (),

    #[webapi(method, length = 0, callback = history_go_callback)]
    go: (),

    #[webapi(method, length = 2, callback = history_push_state_callback)]
    push_state: (),

    #[webapi(method, length = 2, callback = history_replace_state_callback)]
    replace_state: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Navigation, enumerable)]
struct NavigationTemplateMethodsDeclaration {
    #[webapi(method, length = 2, callback = navigation_navigate_callback)]
    navigate: (),

    #[webapi(method, length = 0, callback = navigation_back_callback)]
    back: (),

    #[webapi(method, length = 0, callback = navigation_forward_callback)]
    forward: (),

    #[webapi(method, length = 1, callback = navigation_traverse_to_callback)]
    traverse_to: (),

    #[webapi(method, length = 0, callback = navigation_reload_callback)]
    reload: (),

    #[webapi(method, length = 0, callback = navigation_entries_callback)]
    entries: (),

    #[webapi(method, length = 1, callback = navigation_update_current_entry_callback)]
    update_current_entry: (),
}

pub(in crate::context_bootstrap) fn install_history_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    HistoryTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn install_navigation_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    NavigationTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
}
