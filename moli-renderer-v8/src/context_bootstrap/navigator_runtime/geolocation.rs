use super::super::*;
use crate::web_api_interfaces;
use crate::{
    document_runtime::DomHandle,
    host::HostTimerOwner,
    util::{
        context_host_ptr_from_global_bridge, get_private_value, set_private_value, throw_type_error,
    },
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod position;
mod watches;

const GEOLOCATION_SECURE_CONTEXT_SLOT: &str = "__moliGeolocationSecureContext";
const GEOLOCATION_NEXT_WATCH_ID_SLOT: &str = "__moliGeolocationNextWatchId";
const GEOLOCATION_CHILD_HANDLE_SLOT: &str = "__moliGeolocationChildHandle";
const GEOLOCATION_POSITION_ERROR_CODE_SLOT: &str = "__moliGeolocationPositionErrorCode";
const GEOLOCATION_POSITION_ERROR_MESSAGE_SLOT: &str = "__moliGeolocationPositionErrorMessage";

const PERMISSION_DENIED: u16 = 1;
const POSITION_UNAVAILABLE: u16 = 2;
const TIMEOUT: u16 = 3;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Geolocation)]
struct GeolocationObjectDeclaration {
    #[webapi(slot = GEOLOCATION_SECURE_CONTEXT_SLOT)]
    secure_context: bool,

    #[webapi(slot = GEOLOCATION_NEXT_WATCH_ID_SLOT, value = 1.0)]
    next_watch_id: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Geolocation, enumerable)]
struct GeolocationPrototypeMethodsDeclaration {
    #[webapi(method, length = 1, callback = geolocation_get_current_position_callback)]
    get_current_position: (),

    #[webapi(method, length = 1, callback = geolocation_watch_position_callback)]
    watch_position: (),

    #[webapi(method, length = 1, callback = geolocation_clear_watch_callback)]
    clear_watch: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::GeolocationPositionError)]
struct GeolocationPositionErrorObjectDeclaration {
    #[webapi(slot = GEOLOCATION_POSITION_ERROR_CODE_SLOT)]
    code: u16,

    #[webapi(slot = GEOLOCATION_POSITION_ERROR_MESSAGE_SLOT)]
    message: String,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GeolocationPositionError, enumerable)]
struct GeolocationPositionErrorPrototypeDeclaration {
    #[webapi(accessor_property, getter = geolocation_position_error_code_getter_callback)]
    code: (),

    #[webapi(accessor_property, getter = geolocation_position_error_message_getter_callback)]
    message: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GeolocationPositionError, enumerable)]
struct GeolocationPositionErrorConstantsDeclaration {
    #[webapi(constant = "PERMISSION_DENIED", value = 1u32)]
    permission_denied: (),

    #[webapi(constant = "POSITION_UNAVAILABLE", value = 2u32)]
    position_unavailable: (),

    #[webapi(constant = "TIMEOUT", value = 3u32)]
    timeout: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Geolocation.getCurrentPosition")]
struct GetCurrentPositionArgs {
    #[webidl(required, converter = "callback_function")]
    success_callback: webidl::WebIdlCallbackFunction,

    #[webidl(nullable, converter = "callback_function")]
    error_callback: Option<webidl::WebIdlCallbackFunction>,

    #[webidl(with = position_options_arg)]
    options: PositionOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Geolocation.watchPosition")]
struct WatchPositionArgs {
    #[webidl(required, converter = "callback_function")]
    success_callback: webidl::WebIdlCallbackFunction,

    #[webidl(nullable, converter = "callback_function")]
    error_callback: Option<webidl::WebIdlCallbackFunction>,

    #[webidl(with = position_options_arg)]
    options: PositionOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Geolocation.clearWatch")]
struct ClearWatchArgs {
    #[webidl(required)]
    watch_id: i32,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PositionOptions")]
struct PositionOptions {
    #[webidl(name = "enableHighAccuracy", default = false)]
    enable_high_accuracy: bool,

    #[webidl(with = clamped_unsigned_long_position_option)]
    timeout: u32,

    #[webidl(name = "maximumAge", with = clamped_unsigned_long_position_option)]
    maximum_age: u32,
}

pub(super) fn install_geolocation_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    position::install(scope, template, interface_name);
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Geolocation" => {
            GeolocationPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "GeolocationPositionError" => {
            GeolocationPositionErrorPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            GeolocationPositionErrorConstantsDeclaration::initialize_template(scope, template);
            GeolocationPositionErrorConstantsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

pub(super) fn build_geolocation_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    secure_context: bool,
    owner_child: Option<DomHandle>,
) -> Result<v8::Local<'s, v8::Object>> {
    let geolocation = GeolocationObjectDeclaration::new(secure_context)
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind Geolocation object: {error}"))?;
    let child_handle = owner_child
        .map(|handle| v8::BigInt::new_from_u64(scope, handle.index() as u64).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        geolocation,
        GEOLOCATION_CHILD_HANDLE_SLOT,
        child_handle,
    );
    watches::initialize(scope, geolocation);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.register_geolocation_object(scope, geolocation);
    }
    Ok(geolocation)
}

fn geolocation_get_current_position_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !geolocation_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<GetCurrentPositionArgs>(scope, &args) else {
        return;
    };
    let _ = (
        parsed.options.enable_high_accuracy,
        parsed.options.maximum_age,
    );
    queue_geolocation_result(
        scope,
        args.this(),
        parsed.success_callback,
        parsed.error_callback,
        parsed.options.timeout,
        None,
    );
    rv.set_undefined();
}

fn geolocation_watch_position_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !geolocation_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<WatchPositionArgs>(scope, &args) else {
        return;
    };
    let _ = (
        parsed.options.enable_high_accuracy,
        parsed.options.maximum_age,
    );
    let watch_id = take_next_watch_id(scope, args.this());
    watches::insert(
        scope,
        args.this(),
        watch_id,
        parsed.success_callback,
        parsed.error_callback,
        parsed.options.timeout,
    );
    rv.set_int32(watch_id);
}

fn geolocation_clear_watch_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !geolocation_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ClearWatchArgs>(scope, &args) else {
        return;
    };
    watches::remove(scope, args.this(), parsed.watch_id);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let _ =
            unsafe { &mut *host_ptr }.cancel_geolocation_watch(scope, args.this(), parsed.watch_id);
    }
    rv.set_undefined();
}

fn position_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> std::result::Result<PositionOptions, webidl::WebIdlError> {
    if args.length() <= index || args.get(index).is_undefined() {
        return Ok(PositionOptions {
            timeout: u32::MAX,
            ..PositionOptions::default()
        });
    }
    webidl::parse_dictionary::<PositionOptions>(
        scope,
        args.get(index),
        webidl::Context::argument("Geolocation", (index + 1) as usize),
    )
    .map(|options| {
        options.unwrap_or(PositionOptions {
            timeout: u32::MAX,
            ..PositionOptions::default()
        })
    })
}

fn clamped_unsigned_long_position_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<u32, webidl::WebIdlError> {
    let default = if name == "timeout" { u32::MAX } else { 0 };
    let value = webidl::optional_member::<webidl::UnrestrictedDouble>(
        scope,
        object,
        name,
        webidl::Context::member("PositionOptions", name),
    )?;
    Ok(value.map_or(default, |value| clamp_unsigned_long(value.0)))
}

fn clamp_unsigned_long(value: f64) -> u32 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= f64::from(u32::MAX) {
        return u32::MAX;
    }
    value.round_ties_even() as u32
}

fn geolocation_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::Geolocation::is_instance(scope, receiver)
}

fn take_next_watch_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
) -> i32 {
    let watch_id = get_private_value(scope, geolocation, GEOLOCATION_NEXT_WATCH_ID_SLOT)
        .and_then(|value| value.int32_value(scope))
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let next = watch_id
        .checked_add(1)
        .filter(|value| *value > 0)
        .unwrap_or(1);
    set_private_value(
        scope,
        geolocation,
        GEOLOCATION_NEXT_WATCH_ID_SLOT,
        v8::Integer::new(scope, next).into(),
    );
    watch_id
}

fn queue_geolocation_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
    success_callback: webidl::WebIdlCallbackFunction,
    error_callback: Option<webidl::WebIdlCallbackFunction>,
    timeout: u32,
    watch_id: Option<i32>,
) {
    let (code, message) = geolocation_error(scope, geolocation, timeout);
    if code == PERMISSION_DENIED
        && let Some(watch_id) = watch_id
    {
        // A permission denial is terminal, unlike a temporarily unavailable
        // position. Do not retain the callbacks for future override updates.
        watches::remove(scope, geolocation, watch_id);
    }
    let position = context_host_ptr_from_global_bridge(scope)
        .and_then(|host_ptr| {
            unsafe { &*host_ptr }
                .navigator_overrides()
                .geolocation
                .as_ref()
                .and_then(moli_page_types::GeolocationOverride::position)
                .cloned()
        })
        .filter(|_| code == POSITION_UNAVAILABLE);
    // Without an override Moli has no native location provider yet, so clearing
    // and explicitly simulating an unavailable position both deliver this error.
    // Keep them distinct in host state so source changes still notify watches.
    let callback = if position.is_some() {
        success_callback
    } else if let Some(callback) = error_callback {
        callback
    } else {
        return;
    };
    let Some(geolocation_context) = geolocation.get_creation_context(scope) else {
        return;
    };
    // Both position and error belong to the Geolocation object's relevant
    // Realm, independently of the callback's relevant Realm.
    let result = {
        let scope = &mut v8::ContextScope::new(scope, geolocation_context);
        let result = if let Some(position) = position {
            position::build(scope, &position)
        } else {
            let Ok(error) =
                GeolocationPositionErrorObjectDeclaration::new(code, message.to_owned())
                    .bind(scope)
            else {
                return;
            };
            error
        };
        v8::Global::new(scope, v8::Local::<v8::Value>::from(result))
    };
    let owner = geolocation_child_handle(scope, geolocation)
        .map(HostTimerOwner::ChildWindow)
        .unwrap_or(HostTimerOwner::Window);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let _ = unsafe { &mut *host_ptr }.queue_window_geolocation_callback(
            scope,
            callback,
            geolocation,
            result,
            owner,
            watch_id,
        );
    }
}

pub(crate) fn notify_geolocation_override_changed(scope: &mut v8::PinScope<'_, '_>) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let geolocations = unsafe { &mut *host_ptr }.live_geolocation_objects(scope);
    for geolocation in geolocations {
        let Some(context) = geolocation.get_creation_context(scope) else {
            continue;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        watches::notify(scope, geolocation);
    }
}

fn geolocation_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
    timeout: u32,
) -> (u16, &'static str) {
    let secure_context = get_private_value(scope, geolocation, GEOLOCATION_SECURE_CONTEXT_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    let permission_denied =
        context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| unsafe {
            (&*host_ptr).permission_state("geolocation") == "denied"
        });
    if !secure_context || permission_denied {
        (PERMISSION_DENIED, "Geolocation permission denied")
    } else if timeout == 0 {
        (TIMEOUT, "Geolocation request timed out")
    } else {
        (POSITION_UNAVAILABLE, "Position unavailable")
    }
}

fn geolocation_child_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let value = get_private_value(scope, geolocation, GEOLOCATION_CHILD_HANDLE_SLOT)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (index, lossless) = big.u64_value();
    lossless.then(|| DomHandle::new(index as usize))
}

fn geolocation_position_error_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if web_api_interfaces::GeolocationPositionError::is_instance(scope, receiver) {
        Some(receiver)
    } else {
        throw_type_error(scope, "Illegal invocation");
        None
    }
}

fn geolocation_position_error_code_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = geolocation_position_error_receiver(scope, args.this()) else {
        return;
    };
    let code = get_private_value(scope, receiver, GEOLOCATION_POSITION_ERROR_CODE_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or_default();
    rv.set_uint32(code);
}

fn geolocation_position_error_message_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = geolocation_position_error_receiver(scope, args.this()) else {
        return;
    };
    let message = get_private_value(scope, receiver, GEOLOCATION_POSITION_ERROR_MESSAGE_SLOT)
        .unwrap_or_else(|| v8::String::empty(scope).into());
    rv.set(message);
}
