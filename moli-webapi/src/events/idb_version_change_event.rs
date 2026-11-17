use super::state::{define_event_property, event_private_value, new_event_state};
use super::{EventInit, initialize_event_object, initialize_event_wrapper};
use crate::interfaces as web_api_interfaces;
use moli_v8_util::throw_type_error;
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webidl as webidl;
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

pub fn install_version_change_event_template_bindings<'s>(
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
    if let Some(value) = event_private_value(scope, args.this(), OLD_VERSION_SLOT) {
        rv.set(value);
    }
}

fn new_version_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = event_private_value(scope, args.this(), NEW_VERSION_SLOT) {
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

pub fn idb_version_change_event_constructor_callback<'s>(
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
    let event = new_event_state(scope);
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.init.base.bubbles,
        parsed.init.base.cancelable,
    );
    define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, parsed.init.base.composed).into(),
    );
    parsed
        .init
        .initialize_snapshot(scope, event)
        .expect("IDBVersionChangeEvent state");
    web_api_interfaces::initialize(scope, event, "IDBVersionChangeEvent")
        .expect("IDBVersionChangeEvent brand");
    if initialize_event_wrapper(scope, wrapper, event).is_some() {
        rv.set(wrapper.into());
    }
}

pub fn new_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    old_version: u64,
    new_version: Option<u64>,
) -> v8::Local<'s, v8::Object> {
    let state = new_event_state(scope);
    initialize_event_object(scope, state, event_type, false, false);
    IdbVersionChangeEventInit {
        base: EventInit::default(),
        old_version,
        new_version,
    }
    .initialize_snapshot(scope, state)
    .expect("IDBVersionChangeEvent state");
    web_api_interfaces::initialize(scope, state, "IDBVersionChangeEvent")
        .expect("IDBVersionChangeEvent brand");
    state
}
