//! FetchEvent construction and the state shared with native fetch dispatch.
use moli_webapi::events::fetch::fetch_event_constructor_callback;

use super::*;

const FETCH_EVENT_PROTOTYPE_SLOT: &str = "__moliFetchEventPrototype";

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::FetchEvent,
    constructor_callback = fetch_event_constructor_callback,
    constructor_length = 2,
    readonly_prototype,
    receiver = web_api_interfaces::FetchEvent::is_instance
)]
struct FetchEventTemplateDeclaration {
    #[webapi(method = "respondWith", enumerable, callback = fetch_event_respond_with_callback, length = 1)]
    respond_with: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct FetchEventConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "FetchEvent")]
    fetch_event: v8::Local<'scope, v8::Function>,
}

pub(super) fn install_service_worker_fetch_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let template = FetchEventTemplateDeclaration::build(scope);
    moli_webapi::events::fetch::install_value_template_bindings(scope, template);
    let constructor = template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build FetchEvent constructor"))?;
    let prototype = constructor_prototype(scope, constructor, "FetchEvent")?;
    set_worker_to_string_tag(scope, prototype, "FetchEvent");
    let parent = global_constructor_object(scope, "ExtendableEvent")
        .ok_or_else(|| anyhow!("missing ExtendableEvent constructor"))?;
    let parent_prototype = global_constructor_prototype(scope, "ExtendableEvent")
        .ok_or_else(|| anyhow!("missing ExtendableEvent prototype"))?;
    let _ = constructor.set_prototype(scope, parent.into());
    let _ = prototype.set_prototype(scope, parent_prototype.into());
    // Native dispatch must not consult an author-replaced constructor/getter.
    set_private_value(scope, global, FETCH_EVENT_PROTOTYPE_SLOT, prototype.into());
    FetchEventConstructorGlobalDeclaration::new(constructor)
        .initialize(scope, global)
        .map_err(Into::into)
}

fn fetch_event_respond_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(scope, "FetchEvent.respondWith requires an argument.");
        return;
    }
    crate::worker::thread::service_worker_respond_with_callback(scope, args, rv);
}

pub(in crate::worker) fn initialize_native_fetch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handled: v8::Local<'s, v8::Promise>,
    preload_response: v8::Local<'s, v8::Promise>,
    client_id: &str,
    resulting_client_id: &str,
) {
    let global = scope.get_current_context().global(scope);
    let prototype = get_private_value(scope, global, FETCH_EVENT_PROTOTYPE_SLOT)
        .expect("Service Worker FetchEvent prototype");
    assert_eq!(event.set_prototype(scope, prototype), Some(true));
    crate::context_bootstrap::initialize_event_object(scope, event, "fetch", false, true);
    crate::context_bootstrap::mark_event_trusted(scope, event);
    moli_webapi::events::fetch::initialize_native_fetch_event_values(
        scope,
        event,
        request,
        handled,
        preload_response,
        client_id,
        resulting_client_id,
    );
}
