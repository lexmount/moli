use crate::v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction;
use crate::{
    util::{get_private_value, set_private_value, v8_string_from_utf16_units},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const REGISTERED_OBSERVERS_SLOT: &str = "__moliRegisteredReportingObservers";
const BUFFERED_SLOT: &str = "__moliReportingObserverBuffered";

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ReportingObserverOptions")]
struct ReportingObserverOptions {
    #[webidl(default = false)]
    buffered: bool,
    #[webidl(sequence, converter = "raw", default = Vec::new())]
    types: Vec<webidl::DomString16>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReportingObserver")]
struct ReportingObserverArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
    #[webidl(dictionary)]
    options: ReportingObserverOptions,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ReportingObserver)]
struct ReportingObserverObject<'s> {
    #[webapi(slot = "__moliReportingObserverCallback")]
    callback: v8::Local<'s, v8::Object>,
    #[webapi(slot = "__moliReportingObserverTypes")]
    types: v8::Local<'s, v8::Array>,
    #[webapi(slot = BUFFERED_SLOT)]
    buffered: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReportingObserver, enumerable, receiver)]
struct ReportingObserverMethods {
    #[webapi(method, callback = reporting_observer_observe)]
    observe: (),
    #[webapi(method, callback = reporting_observer_disconnect)]
    disconnect: (),
    #[webapi(method, callback = reporting_observer_take_records)]
    take_records: (),
}

pub(super) fn install_reporting_observer_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    ReportingObserverMethods::initialize_prototype_template(scope, prototype);
}

pub(super) fn reporting_observer_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        crate::context_bootstrap::throw_type_error(scope, "ReportingObserver requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ReportingObserverArgs>(scope, &args) else {
        return;
    };
    let callback = V8TracedWebIdlCallbackFunction::new(scope, parsed.callback).into_object();
    let types = parsed
        .options
        .types
        .iter()
        .map(|value| {
            v8_string_from_utf16_units(scope, &value.0)
                .expect("ReportingObserver type")
                .into()
        })
        .collect::<Vec<v8::Local<v8::Value>>>();
    let types = v8::Array::new_with_elements(scope, &types);
    ReportingObserverObject::new(callback, types, parsed.options.buffered)
        .initialize(scope, args.this())
        .expect("ReportingObserver native state");
    rv.set(args.this().into());
}

fn registered_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Set> {
    // Borrowed methods still register with the observer's relevant global.
    let context = observer
        .get_creation_context(scope)
        .expect("ReportingObserver realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    if let Some(set) = get_private_value(scope, global, REGISTERED_OBSERVERS_SLOT)
        .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok())
    {
        return set;
    }
    let set = v8::Set::new(scope);
    set_private_value(scope, global, REGISTERED_OBSERVERS_SLOT, set.into());
    set
}

fn reporting_observer_observe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let observers = registered_observers(scope, args.this());
    let _ = observers.add(scope, args.this().into());
    // There are no report producers or buffered reports yet. Consume the
    // buffered option once, as observe() requires, without fabricating records.
    set_private_value(
        scope,
        args.this(),
        BUFFERED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    rv.set_undefined();
}

fn reporting_observer_disconnect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let observers = registered_observers(scope, args.this());
    let _ = observers.delete(scope, args.this().into());
    rv.set_undefined();
}

fn reporting_observer_take_records(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Array::new(scope, 0).into());
}
