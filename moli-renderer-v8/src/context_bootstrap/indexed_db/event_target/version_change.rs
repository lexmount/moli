use super::super::super::events;
use super::*;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct IdbVersionChangeEventFieldsDeclaration<'scope> {
    old_version: u64,
    new_version: v8::Local<'scope, v8::Value>,
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
    #[webidl(default = 0)]
    old_version: u64,
    #[webidl(nullable)]
    new_version: Option<u64>,
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
    let event = args.this();
    events::initialize_event_object(scope, event, &parsed.event_type, false, false);
    IdbVersionChangeEventFieldsDeclaration::new(
        parsed.init.old_version,
        version_change_nullable_version_value(scope, parsed.init.new_version),
    )
    .initialize(scope, event)
    .expect("IDBVersionChangeEvent fields declaration should initialize");
    rv.set(event.into());
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_version_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    old_version: u64,
    new_version: Option<u64>,
) -> bool {
    let global = scope.get_current_context().global(scope);
    let Some(event_ctor) = global
        .get(scope, v8str(scope, "IDBVersionChangeEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return dispatch_idb_named_event(scope, target, event_type, |scope, event| {
            IdbVersionChangeEventFieldsDeclaration::new(
                old_version,
                version_change_nullable_version_value(scope, new_version),
            )
            .initialize(scope, event)
            .expect("IDBVersionChangeEvent fallback fields declaration should initialize");
        });
    };
    let Some(event_type) = v8_string(scope, event_type) else {
        return true;
    };
    let Some(event) = event_ctor.new_instance(scope, &[event_type.into()]) else {
        return true;
    };
    IdbVersionChangeEventFieldsDeclaration::new(
        old_version,
        version_change_nullable_version_value(scope, new_version),
    )
    .initialize(scope, event)
    .expect("IDBVersionChangeEvent dispatched fields declaration should initialize");
    dispatch_idb_event_object(scope, target, event)
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
