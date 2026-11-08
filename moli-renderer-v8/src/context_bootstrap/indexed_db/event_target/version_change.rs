use super::super::super::events;
use super::*;
use crate::context_bootstrap::events::EventInit;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiFunctionTemplate;
use webidl::WebIdlDictionarySerialize;

const OLD_VERSION_SLOT: &str = "__moli_idb_event_old_version";
const NEW_VERSION_SLOT: &str = "__moli_idb_event_new_version";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBVersionChangeEvent, enumerable, receiver)]
struct IdbVersionChangeEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = old_version_getter)]
    old_version: (),
    #[webapi(accessor_property, getter = new_version_getter)]
    new_version: (),
}

pub(in crate::context_bootstrap::indexed_db) fn install_version_change_event_template_bindings<
    's,
>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
) {
    if name == "IDBVersionChangeEvent" {
        IdbVersionChangeEventPrototypeDeclaration::initialize_prototype_template(scope, prototype);
    }
}

fn old_version_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = events::event_private_value(scope, args.this(), OLD_VERSION_SLOT) {
        rv.set(value);
    }
}

fn new_version_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = events::event_private_value(scope, args.this(), NEW_VERSION_SLOT) {
        rv.set(value);
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBVersionChangeEvent")]
struct IdbVersionChangeEventConstructorArgs {
    #[webidl(required, name = "type")]
    event_type: String,
    #[webidl(index = 1, dictionary)]
    init: IdbVersionChangeEventInit,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBVersionChangeEventInit", snapshot)]
struct IdbVersionChangeEventInit {
    #[webidl(inherit)]
    base: EventInit,
    // WebIDL reads inherited EventInit members first, then this dictionary's
    // members in lexicographic order. The derive preserves inheritance order.
    #[webidl(nullable)]
    #[webidl(snapshot_slot = NEW_VERSION_SLOT)]
    new_version: Option<u64>,
    #[webidl(default = 0)]
    #[webidl(snapshot_slot = OLD_VERSION_SLOT)]
    old_version: u64,
}

pub(in crate::context_bootstrap) fn idb_version_change_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'IDBVersionChangeEvent': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<IdbVersionChangeEventConstructorArgs>(scope, &args)
    else {
        return;
    };
    let wrapper = args.this();
    let event = events::new_event_state(scope);
    events::initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.init.base.bubbles,
        parsed.init.base.cancelable,
    );
    events::define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, parsed.init.base.composed).into(),
    );
    parsed
        .init
        .initialize_snapshot(scope, event)
        .expect("IDBVersionChangeEvent state");
    crate::web_api_interfaces::initialize(scope, event, "IDBVersionChangeEvent")
        .expect("IDBVersionChangeEvent brand");
    if events::initialize_event_wrapper(scope, wrapper, event).is_some() {
        rv.set(wrapper.into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_version_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    old_version: u64,
    new_version: Option<u64>,
) -> bool {
    dispatch_version_change_event_with_result(scope, target, event_type, old_version, new_version)
        .uncanceled
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_version_change_event_with_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    old_version: u64,
    new_version: Option<u64>,
) -> IdbEventDispatchResult {
    let state = events::new_event_state(scope);
    events::initialize_event_object(scope, state, event_type, false, false);
    IdbVersionChangeEventInit {
        base: EventInit::default(),
        old_version,
        new_version,
    }
    .initialize_snapshot(scope, state)
    .expect("IDBVersionChangeEvent state");
    crate::web_api_interfaces::initialize(scope, state, "IDBVersionChangeEvent")
        .expect("IDBVersionChangeEvent brand");
    let Some(event) = events::new_event_wrapper(scope, state) else {
        return IdbEventDispatchResult {
            uncanceled: true,
            did_listeners_throw: false,
        };
    };
    events::mark_event_trusted(scope, event);
    dispatch_idb_event_object_with_result(scope, target, event)
}
