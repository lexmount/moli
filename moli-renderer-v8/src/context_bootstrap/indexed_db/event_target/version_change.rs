use super::super::super::events;
use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const OLD_VERSION_SLOT: &str = "__moli_idb_event_old_version";
const NEW_VERSION_SLOT: &str = "__moli_idb_event_new_version";

#[derive(WebApiObject)]
#[webapi(fragment)]
struct IdbVersionChangeEventFieldsDeclaration<'scope> {
    #[webapi(slot = OLD_VERSION_SLOT)]
    old_version: u64,
    #[webapi(slot = NEW_VERSION_SLOT)]
    new_version: v8::Local<'scope, v8::Value>,
}

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
    #[webidl(index = 1, with = parse_version_change_event_init_arg)]
    init: IdbVersionChangeEventInit,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBVersionChangeEventInit")]
struct IdbVersionChangeEventInit {
    // WebIDL reads inherited EventInit members first, then this dictionary's
    // members in lexicographic order. The derive preserves declaration order.
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(nullable)]
    new_version: Option<u64>,
    #[webidl(default = 0)]
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
        parsed.init.bubbles,
        parsed.init.cancelable,
    );
    events::define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, parsed.init.composed).into(),
    );
    IdbVersionChangeEventFieldsDeclaration::new(
        parsed.init.old_version,
        version_change_nullable_version_value(scope, parsed.init.new_version),
    )
    .initialize(scope, event)
    .expect("IDBVersionChangeEvent fields declaration should initialize");
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
) -> IdbEventDispatchResult {
    let state = events::new_event_state(scope);
    events::initialize_event_object(scope, state, event_type, false, false);
    IdbVersionChangeEventFieldsDeclaration::new(
        old_version,
        version_change_nullable_version_value(scope, new_version),
    )
    .initialize(scope, state)
    .expect("IDBVersionChangeEvent state");
    crate::web_api_interfaces::initialize(scope, state, "IDBVersionChangeEvent")
        .expect("IDBVersionChangeEvent brand");
    let Some(event) = events::new_event_wrapper(scope, state) else {
        return IdbEventDispatchResult::default();
    };
    events::mark_event_trusted(scope, event);
    dispatch_idb_event_object(scope, target, event, event_type)
}

fn version_change_nullable_version_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    version: Option<u64>,
) -> v8::Local<'s, v8::Value> {
    version
        .map(|value| v8::Number::new(scope, value as f64).into())
        .unwrap_or_else(|| v8::null(scope).into())
}

fn parse_version_change_event_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IdbVersionChangeEventInit, webidl::WebIdlError> {
    let context = webidl::Context::argument("IDBVersionChangeEvent", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|init| init.unwrap_or_default())
}
