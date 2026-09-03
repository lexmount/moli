use super::*;
use crate::context_bootstrap::file_api::is_branded_data_transfer_object;
use crate::context_bootstrap::navigation_activation::install_navigation_transition;
use crate::context_bootstrap::navigation_events::navigation_scroll_event_is_active;
use crate::context_bootstrap::navigation_handler_callbacks::{
    NAVIGATE_EVENT_ADDED_HANDLERS_SLOT, NAVIGATE_EVENT_DEFERRED_HANDLERS_SLOT,
    NAVIGATE_EVENT_PRECOMMIT_HANDLERS_SLOT, navigation_handler_array_is_empty,
    push_navigation_handler, run_navigation_handler_array,
};
use crate::context_bootstrap::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, runtime_window_is_global, runtime_window_owner,
};
use crate::native_bridge::element::scroll_to_url_fragment_or_top;
use crate::native_bridge::throw_dom_exception;
use crate::util::context_host_ptr_from_global_bridge;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use moli_webapi_declare::WebApiObject;
use url::Url;

const NAVIGATE_EVENT_SYNTHETIC_SLOT: &str = "__lmNavigateEventSynthetic";
const NAVIGATE_EVENT_INTERCEPTED_SLOT: &str = "__lmNavigateEventIntercepted";
const NAVIGATION_DESTINATION_STATE_SLOT: &str = "__lmNavigationDestinationState";
const NAVIGATE_EVENT_PRECOMMIT_SEEN_SLOT: &str = "__lmNavigateEventPrecommitSeen";
const NAVIGATE_EVENT_REDIRECTED_SLOT: &str = "__lmNavigateEventRedirected";
const NAVIGATE_EVENT_REDIRECT_HISTORY_SLOT: &str = "__lmNavigateEventRedirectHistory";
const NAVIGATE_EVENT_FOCUS_RESET_SLOT: &str = "__lmNavigateEventFocusReset";
const NAVIGATE_EVENT_SCROLL_CALLED_SLOT: &str = "__lmNavigateEventScrollCalled";
const NAVIGATE_EVENT_SCROLL_AFTER_TRANSITION_SLOT: &str = "__lmNavigateEventScrollAfterTransition";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_NAVIGATION_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionNavigation";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_FROM_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionFrom";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_DESTINATION_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionDestination";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_TYPE_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionType";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionResolver";
const PRECOMMIT_CONTROLLER_EVENT_SLOT: &str = "__lmPrecommitControllerEvent";
const PRECOMMIT_CONTROLLER_ACTIVE_SLOT: &str = "__lmPrecommitControllerActive";

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "NavigationFocusReset", rename_all = "kebab-case")]
enum NavigationFocusReset {
    #[default]
    AfterTransition,
    Manual,
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "NavigationScrollBehavior", rename_all = "kebab-case")]
enum NavigationScrollBehavior {
    #[default]
    AfterTransition,
    Manual,
}

/// The four members are declared in Web IDL lexicographic order. Conversion
/// completes before `intercept()` mutates the NavigateEvent, so a getter or
/// callback conversion failure cannot leave a partially intercepted event.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigateEvent.intercept")]
struct NavigationInterceptOptionsMembers {
    #[webidl(name = "focusReset", converter = "enum")]
    focus_reset: Option<NavigationFocusReset>,
    #[webidl(converter = "callback_function")]
    handler: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "precommitHandler", converter = "callback_function")]
    precommit_handler: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "enum")]
    scroll: Option<NavigationScrollBehavior>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct PrecommitControllerDeclaration<'scope> {
    #[webapi(slot = PRECOMMIT_CONTROLLER_EVENT_SLOT)]
    event: v8::Local<'scope, v8::Object>,
    #[webapi(slot = PRECOMMIT_CONTROLLER_ACTIVE_SLOT)]
    active: bool,
    #[webapi(
        method,
        length = 1,
        enumerable,
        callback = precommit_controller_add_handler_callback,
        data = object
    )]
    add_handler: (),
    #[webapi(
        method,
        length = 1,
        enumerable,
        callback = precommit_controller_redirect_callback,
        data = object
    )]
    redirect: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "NavigateEvent")]
struct NavigateEventMethodsDeclaration {
    #[webapi(
        method,
        length = 0,
        enumerable,
        callback = navigate_event_intercept_callback,
        data = object
    )]
    intercept: (),
    #[webapi(
        method,
        length = 0,
        enumerable,
        callback = navigate_event_defer_page_swap_callback,
        data = object
    )]
    defer_page_swap: (),
    #[webapi(
        method,
        length = 0,
        enumerable,
        callback = navigate_event_scroll_callback,
        data = object
    )]
    scroll: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct DragEventInitDeclaration<'scope> {
    data_transfer: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct ClipboardEventInitDeclaration<'scope> {
    clipboard_data: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct CapturedMouseEventInitDeclaration {
    surface_x: i32,
    surface_y: i32,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct MessageEventInitDeclaration<'scope> {
    data: v8::Local<'scope, v8::Value>,
    origin: v8::Local<'scope, v8::String>,
    last_event_id: v8::Local<'scope, v8::String>,
    source: v8::Local<'scope, v8::Value>,
    ports: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct InputEventInitDeclaration<'scope> {
    data: v8::Local<'scope, v8::Value>,
    input_type: v8::Local<'scope, v8::String>,
    is_composing: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct InterestEventInitDeclaration<'scope> {
    source: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ToggleEventStateDeclaration {
    #[webapi(data_property = "oldState", readonly, dont_delete)]
    old_state: String,
    #[webapi(data_property = "newState", readonly, dont_delete)]
    new_state: String,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct PopStateEventInitDeclaration<'scope> {
    state: v8::Local<'scope, v8::Value>,
    #[webapi(data_property = "hasUAVisualTransition")]
    has_ua_visual_transition: bool,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PopStateEventInit")]
struct PopStateEventInitMembers<'s> {
    #[webidl(name = "hasUAVisualTransition", default = false)]
    has_ua_visual_transition: bool,
    #[webidl(converter = "raw")]
    state: Option<v8::Local<'s, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct PageTransitionEventOwnInitDeclaration {
    persisted: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct ErrorEventInitDeclaration<'scope> {
    message: v8::Local<'scope, v8::String>,
    filename: v8::Local<'scope, v8::String>,
    lineno: f64,
    colno: f64,
    error: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct PromiseRejectionEventInitDeclaration<'scope> {
    promise: v8::Local<'scope, v8::Value>,
    reason: v8::Local<'scope, v8::Value>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PromiseRejectionEventInit")]
struct PromiseRejectionEventInitMembers<'s> {
    #[webidl(required)]
    promise: v8::Local<'s, v8::Promise>,
    #[webidl(converter = "raw")]
    reason: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SecurityPolicyViolationEventInit")]
struct SecurityPolicyViolationEventInitMembers {
    #[webidl(name = "documentURI", default = "")]
    document_uri: String,
    #[webidl(default = "")]
    referrer: String,
    #[webidl(name = "blockedURI", default = "")]
    blocked_uri: String,
    #[webidl(name = "violatedDirective", default = "")]
    violated_directive: String,
    #[webidl(name = "effectiveDirective", default = "")]
    effective_directive: String,
    #[webidl(name = "originalPolicy", default = "")]
    original_policy: String,
    #[webidl(default = "enforce")]
    disposition: String,
    #[webidl(name = "sourceFile", default = "")]
    source_file: String,
    #[webidl(default = "")]
    sample: String,
    #[webidl(name = "statusCode", converter = "unsigned_short", default = 0)]
    status_code: u16,
    #[webidl(name = "lineNumber", converter = "long", default = 0)]
    line_number: i32,
    #[webidl(name = "columnNumber", converter = "long", default = 0)]
    column_number: i32,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct NavigationCurrentEntryChangeEventInitDeclaration<'scope> {
    from: v8::Local<'scope, v8::Value>,
    navigation_type: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct NavigateEventInitDeclaration<'scope> {
    navigation_type: v8::Local<'scope, v8::String>,
    destination: v8::Local<'scope, v8::Value>,
    can_intercept: bool,
    user_initiated: bool,
    hash_change: bool,
    signal: v8::Local<'scope, v8::Value>,
    form_data: v8::Local<'scope, v8::Value>,
    download_request: v8::Local<'scope, v8::Value>,
    info: v8::Local<'scope, v8::Value>,
    #[webapi(data_property = "hasUAVisualTransition")]
    has_ua_visual_transition: bool,
    source_element: v8::Local<'scope, v8::Value>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ToggleEventInit")]
struct ToggleEventInitMembers<'s> {
    #[webidl(default = "")]
    old_state: String,
    #[webidl(default = "")]
    new_state: String,
    #[webidl(with = toggle_event_source_member)]
    source: Option<v8::Local<'s, v8::Value>>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "CommandEventInit")]
struct CommandEventInitMembers<'s> {
    #[webidl(default = "")]
    command: String,
    #[webidl(converter = "raw")]
    source: Option<v8::Local<'s, v8::Value>>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "StorageEventInit")]
struct StorageEventInitMembers<'s> {
    #[webidl(nullable)]
    key: Option<String>,
    #[webidl(name = "oldValue", nullable)]
    old_value: Option<String>,
    #[webidl(name = "newValue", nullable)]
    new_value: Option<String>,
    #[webidl(default = "", converter = "usv_string")]
    url: String,
    #[webidl(name = "storageArea", converter = "raw", nullable)]
    storage_area: Option<v8::Local<'s, v8::Value>>,
}

fn toggle_event_source_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<v8::Local<'s, v8::Value>>, webidl::WebIdlError> {
    let context = webidl::Context::member("ToggleEventInit", name);
    match webidl::property_result(scope, object, name, context)? {
        Some(value) if value.is_undefined() => Ok(None),
        Some(value) => Ok(Some(value)),
        None => Ok(None),
    }
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_drag_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    if !pointer::initialize_mouse_event(scope, event, init) {
        return false;
    }
    let data_transfer = match init {
        None => v8::null(scope).into(),
        Some(init) => match webidl::property_result(
            scope,
            init,
            "dataTransfer",
            webidl::Context::member("DragEventInit", "dataTransfer"),
        ) {
            Err(error) => {
                webidl::throw_error(scope, &error);
                return false;
            }
            Ok(None) => v8::null(scope).into(),
            Ok(Some(value)) if value.is_null_or_undefined() => v8::null(scope).into(),
            Ok(Some(value)) => {
                let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
                    throw_type_error(
                        scope,
                        "Failed to construct 'DragEvent': member dataTransfer is not of type DataTransfer.",
                    );
                    return false;
                };
                if !is_branded_data_transfer_object(scope, object) {
                    throw_type_error(
                        scope,
                        "Failed to construct 'DragEvent': member dataTransfer is not of type DataTransfer.",
                    );
                    return false;
                }
                value
            }
        },
    };
    DragEventInitDeclaration::new(data_transfer)
        .initialize(scope, event)
        .expect("DragEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_clipboard_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let clipboard_data =
        init_value_property(scope, init, "clipboardData").unwrap_or_else(|| v8::null(scope).into());
    ClipboardEventInitDeclaration::new(clipboard_data)
        .initialize(scope, event)
        .expect("ClipboardEvent init declaration should initialize");
}

fn captured_mouse_coordinate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    name: &'static str,
) -> Option<i32> {
    let Some(init) = init else {
        return Some(-1);
    };
    let context = webidl::Context::member("CapturedMouseEventInit", name);
    let value = match webidl::property_result(scope, init, name, context) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(-1),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let value = match webidl::convert::<webidl::EnforceRangeLong>(scope, value, context) {
        Ok(value) => i32::from(value),
        Err(error) if error.is_pending_exception() => return None,
        Err(error) => {
            crate::util::throw_range_error(scope, &error.to_string());
            return None;
        }
    };
    if value < -1 {
        crate::util::throw_range_error(
            scope,
            "CapturedMouseEvent coordinates must be -1 or non-negative.",
        );
        return None;
    }
    Some(value)
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_captured_mouse_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(surface_x) = captured_mouse_coordinate(scope, init, "surfaceX") else {
        return false;
    };
    let Some(surface_y) = captured_mouse_coordinate(scope, init, "surfaceY") else {
        return false;
    };
    if (surface_x == -1) != (surface_y == -1) {
        crate::util::throw_range_error(
            scope,
            "CapturedMouseEvent coordinates must both be -1 or both be non-negative.",
        );
        return false;
    }
    CapturedMouseEventInitDeclaration::new(surface_x, surface_y)
        .initialize(scope, event)
        .expect("CapturedMouseEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let data = init
        .and_then(|object| crate::webidl::property(scope, object, "data"))
        .unwrap_or_else(|| v8::null(scope).into());
    let origin = init_string_property(scope, init, "origin", "");
    let origin_value = v8_string(scope, &origin).unwrap();
    let last_event_id = init_string_property(scope, init, "lastEventId", "");
    let last_event_id_value = v8_string(scope, &last_event_id).unwrap();
    let source =
        init_value_property(scope, init, "source").unwrap_or_else(|| v8::null(scope).into());
    let ports = init_value_property(scope, init, "ports");
    let ports = frozen_message_event_ports(scope, ports);
    MessageEventInitDeclaration::new(data, origin_value, last_event_id_value, source, ports)
        .initialize(scope, event)
        .expect("MessageEvent init declaration should initialize");
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_storage_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let parsed = match init {
        Some(init) => {
            match webidl::parse_dictionary_object::<StorageEventInitMembers>(scope, init) {
                Ok(parsed) => parsed,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return false;
                }
            }
        }
        None => StorageEventInitMembers::default(),
    };
    crate::context_bootstrap::events::define_storage_event_properties(
        scope,
        event,
        parsed.key.as_deref(),
        parsed.old_value.as_deref(),
        parsed.new_value.as_deref(),
        &parsed.url,
        parsed.storage_area,
    );
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    basic::initialize_ui_event(scope, event, init);
    let data = init_value_property(scope, init, "data").unwrap_or_else(|| v8::null(scope).into());
    let input_type = init_string_property(scope, init, "inputType", "");
    let input_type_value = v8_string(scope, &input_type).expect("input event inputType");
    let is_composing = init_bool_property(scope, init, "isComposing", false);
    InputEventInitDeclaration::new(data, input_type_value, is_composing)
        .initialize(scope, event)
        .expect("InputEvent init declaration should initialize");
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_pop_state_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let parsed = match init {
        Some(init) => {
            match webidl::parse_dictionary_object::<PopStateEventInitMembers>(scope, init) {
                Ok(parsed) => parsed,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return false;
                }
            }
        }
        None => PopStateEventInitMembers::default(),
    };
    let state = parsed.state.unwrap_or_else(|| v8::null(scope).into());
    PopStateEventInitDeclaration::new(state, parsed.has_ua_visual_transition)
        .initialize(scope, event)
        .expect("PopStateEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_page_transition_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let persisted = init_bool_property(scope, init, "persisted", false);
    PageTransitionEventOwnInitDeclaration::new(persisted)
        .initialize(scope, event)
        .expect("PageTransitionEvent init declaration should initialize");
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_toggle_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let parsed = match init {
        Some(init) => {
            match webidl::parse_dictionary_object::<ToggleEventInitMembers>(scope, init) {
                Ok(parsed) => parsed,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return false;
                }
            }
        }
        None => ToggleEventInitMembers {
            old_state: String::new(),
            new_state: String::new(),
            source: None,
        },
    };
    let source = parsed.source.unwrap_or_else(|| v8::null(scope).into());
    let _ = ToggleEventStateDeclaration::new(parsed.old_state, parsed.new_state)
        .initialize(scope, event);
    set_private_value(scope, event, TOGGLE_EVENT_SOURCE_SLOT, source);
    true
}

fn frozen_message_event_ports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Value> {
    let Some(value) = value.filter(|value| value.is_array()) else {
        let ports = v8::Array::new(scope, 0);
        let _ = ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
        return ports.into();
    };
    let Some(source) = v8::Local::<v8::Array>::try_from(value).ok() else {
        let ports = v8::Array::new(scope, 0);
        let _ = ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
        return ports.into();
    };
    let ports = v8::Array::new(scope, source.length() as i32);
    for index in 0..source.length() {
        if let Some(port) = source.get_index(scope, index) {
            let _ = ports.set_index(scope, index, port);
        }
    }
    let _ = ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    ports.into()
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let message = init_string_property(scope, init, "message", "");
    let message_value = v8_string(scope, &message).expect("ErrorEvent message");
    let filename = init_string_property(scope, init, "filename", "");
    let filename_value = v8_string(scope, &filename).expect("ErrorEvent filename");
    let lineno = init_number_property(scope, init, "lineno", 0.0);
    let colno = init_number_property(scope, init, "colno", 0.0);
    let error = init_value_property(scope, init, "error");
    ErrorEventInitDeclaration::new(message_value, filename_value, lineno, colno, error)
        .initialize(scope, event)
        .expect("ErrorEvent init declaration should initialize");
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_promise_rejection_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let parsed = match init {
        Some(init) => {
            match webidl::parse_dictionary_object::<PromiseRejectionEventInitMembers>(scope, init) {
                Ok(parsed) => parsed,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return false;
                }
            }
        }
        None => {
            let error = webidl::WebIdlError::missing_required(webidl::Context::member(
                "PromiseRejectionEventInit",
                "promise",
            ));
            webidl::throw_error(scope, &error);
            return false;
        }
    };
    let reason = parsed.reason.unwrap_or_else(|| v8::undefined(scope).into());
    PromiseRejectionEventInitDeclaration::new(parsed.promise.into(), reason)
        .initialize(scope, event)
        .expect("PromiseRejectionEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_security_policy_violation_event<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let parsed = match init {
        Some(init) => {
            match webidl::parse_dictionary_object::<SecurityPolicyViolationEventInitMembers>(
                scope, init,
            ) {
                Ok(parsed) => parsed,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return false;
                }
            }
        }
        None => SecurityPolicyViolationEventInitMembers {
            document_uri: String::new(),
            referrer: String::new(),
            blocked_uri: String::new(),
            violated_directive: String::new(),
            effective_directive: String::new(),
            original_policy: String::new(),
            disposition: "enforce".to_owned(),
            source_file: String::new(),
            sample: String::new(),
            status_code: 0,
            line_number: 0,
            column_number: 0,
        },
    };
    let disposition = match parsed.disposition.as_str() {
        "enforce" => crate::content_security_policy::ContentSecurityPolicyDisposition::Enforce,
        "report" => crate::content_security_policy::ContentSecurityPolicyDisposition::Report,
        _ => {
            throw_type_error(
                scope,
                "Failed to construct 'SecurityPolicyViolationEvent': disposition is not a valid SecurityPolicyViolationEventDisposition.",
            );
            return false;
        }
    };
    crate::content_security_policy::initialize_security_policy_violation_event(
        scope,
        event,
        &crate::content_security_policy::ContentSecurityPolicyViolationEventFields {
            document_uri: &parsed.document_uri,
            referrer: &parsed.referrer,
            blocked_uri: &parsed.blocked_uri,
            effective_directive: &parsed.effective_directive,
            violated_directive: &parsed.violated_directive,
            original_policy: &parsed.original_policy,
            disposition,
            source_file: &parsed.source_file,
            sample: &parsed.sample,
            line_number: parsed.line_number,
            column_number: parsed.column_number,
            status_code: i32::from(parsed.status_code),
        },
    )
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_navigation_current_entry_change_event<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(init) = init else {
        throw_type_error(
            scope,
            "Failed to construct 'NavigationCurrentEntryChangeEvent': NavigationCurrentEntryChangeEventInit.from is required.",
        );
        return false;
    };
    let Some(from) = init_value_property(scope, Some(init), "from") else {
        throw_type_error(
            scope,
            "Failed to construct 'NavigationCurrentEntryChangeEvent': NavigationCurrentEntryChangeEventInit.from is required.",
        );
        return false;
    };
    let navigation_type = init_value_property(scope, Some(init), "navigationType")
        .unwrap_or_else(|| v8::null(scope).into());
    NavigationCurrentEntryChangeEventInitDeclaration::new(from, navigation_type)
        .initialize(scope, event)
        .expect("NavigationCurrentEntryChangeEvent init declaration should initialize");
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(init) = init else {
        throw_type_error(
            scope,
            "Failed to construct 'NavigateEvent': NavigateEventInit.destination is required.",
        );
        return false;
    };
    let Some(destination) = init_value_property(scope, Some(init), "destination") else {
        throw_type_error(
            scope,
            "Failed to construct 'NavigateEvent': NavigateEventInit.destination is required.",
        );
        return false;
    };
    let Some(signal) = init_value_property(scope, Some(init), "signal") else {
        throw_type_error(
            scope,
            "Failed to construct 'NavigateEvent': NavigateEventInit.signal is required.",
        );
        return false;
    };

    let navigation_type = init_string_property(scope, Some(init), "navigationType", "push");
    let can_intercept = init_bool_property(scope, Some(init), "canIntercept", false);
    let user_initiated = init_bool_property(scope, Some(init), "userInitiated", false);
    let hash_change = init_bool_property(scope, Some(init), "hashChange", false);
    let has_ua_visual_transition =
        init_bool_property(scope, Some(init), "hasUAVisualTransition", false);
    let form_data = init_value_property(scope, Some(init), "formData")
        .unwrap_or_else(|| v8::null(scope).into());
    let download_request = init_value_property(scope, Some(init), "downloadRequest")
        .unwrap_or_else(|| v8::null(scope).into());
    let info = init
        .get(scope, v8str(scope, "info").into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let source_element = init_value_property(scope, Some(init), "sourceElement")
        .unwrap_or_else(|| v8::null(scope).into());

    let navigation_type = v8_string(scope, &navigation_type).expect("NavigateEvent navigationType");
    NavigateEventInitDeclaration::new(
        navigation_type,
        destination,
        can_intercept,
        user_initiated,
        hash_change,
        signal,
        form_data,
        download_request,
        info,
        has_ua_visual_transition,
        source_element,
    )
    .initialize(scope, event)
    .expect("NavigateEvent init declaration should initialize");
    define_navigate_event_internal_flag(scope, event, true);
    NavigateEventMethodsDeclaration::default()
        .initialize(scope, event)
        .expect("NavigateEvent methods declaration should initialize");
    true
}

fn define_navigate_event_internal_flag(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    synthetic: bool,
) {
    set_private_value(
        scope,
        event,
        NAVIGATE_EVENT_SYNTHETIC_SLOT,
        v8::Boolean::new(scope, synthetic).into(),
    );
}

fn navigate_event_private_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, event, slot).filter(|value| !value.is_undefined())
}

fn navigate_event_private_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
    default: bool,
) -> bool {
    navigate_event_private_value(scope, event, slot)
        .map(|value| value.is_true())
        .unwrap_or(default)
}

fn set_navigate_event_private_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    set_private_value(scope, event, slot, v8::Boolean::new(scope, value).into());
}

fn navigate_event_can_use_navigation_api<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    let synthetic = navigate_event_private_bool(scope, event, NAVIGATE_EVENT_SYNTHETIC_SLOT, true);
    let can_intercept = object_bool_property(scope, event, "canIntercept").unwrap_or(false);
    !synthetic && can_intercept
}

fn navigate_event_throw_synthetic_security_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "SecurityError",
        18,
        "This NavigateEvent was not created by an ongoing navigation.",
    );
}

fn navigate_event_throw_invalid_state(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "This NavigateEvent is no longer dispatching or its navigation target is detached.",
    );
}

fn navigate_event_is_dispatching<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    event_is_dispatching(scope, event)
}

fn navigate_event_target_is_connected(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(target) = event
        .get(scope, v8str(scope, "target").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return true;
    };
    let owner = runtime_window_owner(scope, target);
    if runtime_window_is_global(scope, owner) {
        return true;
    }
    let Some(child_handle) = child_browsing_context_handle_for_runtime_owner(scope, owner) else {
        return true;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return true;
    };
    unsafe { &*host_ptr }.dom_host().is_connected(child_handle)
}

fn navigate_event_can_intercept_now<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    if !navigate_event_is_dispatching(scope, event) {
        return false;
    }
    if object_bool_property(scope, event, "defaultPrevented").unwrap_or(false) {
        return false;
    }
    navigate_event_target_is_connected(scope, event)
        || navigate_event_private_bool(scope, event, NAVIGATE_EVENT_INTERCEPTED_SLOT, false)
}

fn navigate_event_intercept_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(event) = v8::Local::<v8::Object>::try_from(args.data()) else {
        navigate_event_throw_synthetic_security_error(scope);
        return;
    };
    let options = match webidl::parse_dictionary::<NavigationInterceptOptionsMembers>(
        scope,
        args.get(0),
        webidl::Context::argument("NavigateEvent.intercept", 1),
    ) {
        Ok(options) => options.unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !navigate_event_can_use_navigation_api(scope, event) {
        navigate_event_throw_synthetic_security_error(scope);
        return;
    }
    if !navigate_event_can_intercept_now(scope, event) {
        navigate_event_throw_invalid_state(scope);
        return;
    }
    if options.precommit_handler.is_some()
        && !object_bool_property(scope, event, "cancelable").unwrap_or(false)
    {
        navigate_event_throw_invalid_state(scope);
        return;
    }
    set_navigate_event_private_bool(scope, event, NAVIGATE_EVENT_INTERCEPTED_SLOT, true);
    if let Some(focus_reset) = options.focus_reset {
        set_navigate_event_private_bool(
            scope,
            event,
            NAVIGATE_EVENT_FOCUS_RESET_SLOT,
            matches!(focus_reset, NavigationFocusReset::AfterTransition),
        );
    }
    if let Some(scroll) = options.scroll {
        set_navigate_event_private_bool(
            scope,
            event,
            NAVIGATE_EVENT_SCROLL_AFTER_TRANSITION_SLOT,
            matches!(scroll, NavigationScrollBehavior::AfterTransition),
        );
    }
    if let Some(precommit_handler) = options.precommit_handler {
        set_navigate_event_private_bool(scope, event, NAVIGATE_EVENT_PRECOMMIT_SEEN_SLOT, true);
        if push_navigation_handler(
            scope,
            event,
            NAVIGATE_EVENT_PRECOMMIT_HANDLERS_SLOT,
            precommit_handler,
        )
        .is_err()
        {
            return;
        }
    }
    if let Some(handler) = options.handler {
        // This is the final intercept step. A failed residence write leaves a
        // JavaScript exception pending as this callback returns.
        let _residence =
            push_navigation_handler(scope, event, NAVIGATE_EVENT_DEFERRED_HANDLERS_SLOT, handler);
    }
}

/// Runs precommit handlers only after the complete `navigate` event dispatch.
///
/// The controller stays active for the synchronous callback invocations and is
/// retired before any returned Promise reactions run. The navigation
/// transaction, not this helper, owns waiting and commit.
pub(in crate::context_bootstrap) fn run_navigate_event_precommit_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> (
    Option<v8::Local<'s, v8::Value>>,
    Option<v8::Local<'s, v8::Value>>,
) {
    if navigation_handler_array_is_empty(scope, event, NAVIGATE_EVENT_PRECOMMIT_HANDLERS_SLOT) {
        return (None, None);
    }
    install_navigate_event_precommit_transition(scope, event);
    let controller = create_precommit_controller(scope, event);
    let arguments = [controller.into()];
    let result = run_navigation_handler_array(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_HANDLERS_SLOT,
        &arguments,
    );
    set_precommit_controller_active(scope, controller, false);
    result
}

fn install_navigate_event_precommit_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    if get_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT,
    )
    .is_some_and(|value| !value.is_undefined())
    {
        return;
    }
    let Some(navigation) = get_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_NAVIGATION_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()) else {
        return;
    };
    let Some(from) = get_private_value(scope, event, NAVIGATE_EVENT_PRECOMMIT_TRANSITION_FROM_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let destination = get_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_DESTINATION_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let navigation_type =
        get_private_value(scope, event, NAVIGATE_EVENT_PRECOMMIT_TRANSITION_TYPE_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "push".to_owned());
    let navigation_type = match navigation_type.as_str() {
        "replace" => "replace",
        "reload" => "reload",
        "traverse" => "traverse",
        _ => "push",
    };
    let Some(resolver) =
        install_navigation_transition(scope, navigation, from, destination, navigation_type)
    else {
        return;
    };
    set_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT,
        resolver.into(),
    );
}

fn create_precommit_controller<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    PrecommitControllerDeclaration::new(event, true)
        .bind(scope)
        .expect("precommit controller declaration should bind")
}

fn set_precommit_controller_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    controller: v8::Local<'s, v8::Object>,
    active: bool,
) {
    set_private_value(
        scope,
        controller,
        PRECOMMIT_CONTROLLER_ACTIVE_SLOT,
        v8::Boolean::new(scope, active).into(),
    );
}

fn precommit_controller_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let controller = v8::Local::<v8::Object>::try_from(data).ok()?;
    if !get_private_value(scope, controller, PRECOMMIT_CONTROLLER_ACTIVE_SLOT)
        .is_some_and(|value| value.is_true())
    {
        navigate_event_throw_invalid_state(scope);
        return None;
    }
    let event = get_private_value(scope, controller, PRECOMMIT_CONTROLLER_EVENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    if !navigate_event_target_is_connected(scope, event) {
        navigate_event_throw_invalid_state(scope);
        return None;
    }
    Some(event)
}

fn precommit_controller_add_handler_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let handler = match webidl::convert::<webidl::WebIdlCallbackFunction>(
        scope,
        args.get(0),
        webidl::Context::argument("NavigationPrecommitController.addHandler", 1),
    ) {
        Ok(handler) => handler,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Some(event) = precommit_controller_event(scope, args.data()) else {
        return;
    };
    // This is the final addHandler step. A failed residence write leaves a
    // JavaScript exception pending as this callback returns.
    let _residence =
        push_navigation_handler(scope, event, NAVIGATE_EVENT_ADDED_HANDLERS_SLOT, handler);
}

fn precommit_controller_redirect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(event) = precommit_controller_event(scope, args.data()) else {
        return;
    };
    let navigation_type = event
        .get(scope, v8str(scope, "navigationType").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if matches!(navigation_type.as_str(), "reload" | "traverse") {
        navigate_event_throw_invalid_state(scope);
        return;
    }
    let Some(destination) = event
        .get(scope, v8str(scope, "destination").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        navigate_event_throw_invalid_state(scope);
        return;
    };
    let base = destination
        .get(scope, v8str(scope, "url").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let Some(raw_url) = args.get(0).to_string(scope) else {
        return;
    };
    let raw_url = raw_url.to_rust_string_lossy(scope);
    let base_url = match Url::parse(&base) {
        Ok(url) => url,
        Err(_) => {
            navigate_event_throw_invalid_state(scope);
            return;
        }
    };
    let redirected = match base_url.join(&raw_url) {
        Ok(url) => url,
        Err(_) => {
            throw_dom_exception(
                scope,
                "SyntaxError",
                12,
                "Failed to execute 'redirect' on 'NavigationPrecommitController': Invalid URL.",
            );
            return;
        }
    };
    if !moli_url::same_origin(&base_url, &redirected) {
        throw_dom_exception(
            scope,
            "SecurityError",
            18,
            "Failed to execute 'redirect' on 'NavigationPrecommitController': Cannot redirect to a cross-origin URL.",
        );
        return;
    }
    let options_value = args.get(1);
    if !options_value.is_null_or_undefined()
        && let Some(options) = options_value.to_object(scope)
        && !apply_precommit_redirect_options(scope, event, destination, options)
    {
        return;
    }
    define_non_enumerable_string_property(scope, destination, "url", redirected.as_str());
    set_navigate_event_private_bool(scope, event, NAVIGATE_EVENT_REDIRECTED_SLOT, true);
}

fn apply_precommit_redirect_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    destination: v8::Local<'s, v8::Object>,
    options: v8::Local<'s, v8::Object>,
) -> bool {
    if let Some(info) = options.get(scope, v8str(scope, "info").into())
        && !info.is_undefined()
    {
        define_event_property(scope, event, "info", info);
    }
    if let Some(state) = options.get(scope, v8str(scope, "state").into())
        && !state.is_undefined()
    {
        let Some(cloned) = structured_clone_value(scope, state) else {
            return false;
        };
        set_private_value(
            scope,
            destination,
            NAVIGATION_DESTINATION_STATE_SLOT,
            cloned,
        );
    }
    if let Some(history) = options
        .get(scope, v8str(scope, "history").into())
        .filter(|value| !value.is_undefined())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| matches!(value.as_str(), "push" | "replace"))
    {
        set_private_value(
            scope,
            event,
            NAVIGATE_EVENT_REDIRECT_HISTORY_SLOT,
            v8_string(scope, &history).unwrap().into(),
        );
    }
    true
}

fn navigate_event_defer_page_swap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(event) = v8::Local::<v8::Object>::try_from(args.data()) else {
        navigate_event_throw_synthetic_security_error(scope);
        return;
    };
    if !navigate_event_can_use_navigation_api(scope, event) {
        navigate_event_throw_synthetic_security_error(scope);
        return;
    }
    if !navigate_event_can_intercept_now(scope, event) {
        navigate_event_throw_invalid_state(scope);
    }
}

fn navigate_event_scroll_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(event) = v8::Local::<v8::Object>::try_from(args.data()) else {
        navigate_event_throw_synthetic_security_error(scope);
        return;
    };
    if !navigate_event_can_use_navigation_api(scope, event) {
        navigate_event_throw_synthetic_security_error(scope);
        return;
    }
    let intercepted =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_INTERCEPTED_SLOT, false);
    let default_prevented = object_bool_property(scope, event, "defaultPrevented").unwrap_or(false);
    let already_scrolled =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_SCROLL_CALLED_SLOT, false);
    let active = event
        .get(scope, v8str(scope, "target").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .is_some_and(|navigation| navigation_scroll_event_is_active(scope, navigation, event));
    if !intercepted
        || default_prevented
        || already_scrolled
        || !active
        || navigate_event_is_dispatching(scope, event)
    {
        navigate_event_throw_invalid_state(scope);
        return;
    }
    set_navigate_event_private_bool(scope, event, NAVIGATE_EVENT_SCROLL_CALLED_SLOT, true);
    let Some(target_url) = event
        .get(scope, v8str(scope, "destination").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|destination| destination.get(scope, v8str(scope, "url").into()))
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    if let Err(error) = scroll_to_url_fragment_or_top(scope, host_ptr, &target_url)
        && let Some(message) = v8_string(
            scope,
            &format!("Layout failed while scrolling navigation: {error}"),
        )
    {
        let exception = v8::Exception::error(scope, message);
        scope.throw_exception(exception);
    }
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_close_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let was_clean = init_bool_property(scope, init, "wasClean", false);
    let code = init_number_property(scope, init, "code", 0.0);
    let reason = init_string_property(scope, init, "reason", "");
    let reason_value = v8_string(scope, &reason).unwrap().into();
    set_private_value(
        scope,
        event,
        CLOSE_EVENT_WAS_CLEAN_SLOT,
        v8::Boolean::new(scope, was_clean).into(),
    );
    set_private_value(
        scope,
        event,
        CLOSE_EVENT_CODE_SLOT,
        v8::Number::new(scope, code).into(),
    );
    set_private_value(scope, event, CLOSE_EVENT_REASON_SLOT, reason_value);
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_submit_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(submitter) = submit_event_submitter(scope, init) else {
        return false;
    };
    set_private_value(scope, event, SUBMIT_EVENT_SUBMITTER_SLOT, submitter);
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_form_data_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let Some(init) = init else {
        throw_type_error(
            scope,
            "Failed to construct 'FormDataEvent': FormDataEventInit.formData is required.",
        );
        return false;
    };
    let Some(form_data) = init_value_property(scope, Some(init), "formData") else {
        throw_type_error(
            scope,
            "Failed to construct 'FormDataEvent': FormDataEventInit.formData is required.",
        );
        return false;
    };
    let Ok(form_data_object) = v8::Local::<v8::Object>::try_from(form_data) else {
        throw_type_error(
            scope,
            "Failed to construct 'FormDataEvent': formData must be a FormData object.",
        );
        return false;
    };
    if !crate::context_bootstrap::form_data_runtime::form_data_is_object(scope, form_data_object) {
        throw_type_error(
            scope,
            "Failed to construct 'FormDataEvent': formData must be a FormData object.",
        );
        return false;
    }
    set_private_value(scope, event, FORM_DATA_EVENT_FORM_DATA_SLOT, form_data);
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_command_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let parsed = match init {
        Some(init) => match webidl::parse_dictionary_object::<CommandEventInitMembers>(scope, init)
        {
            Ok(parsed) => parsed,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return false;
            }
        },
        None => CommandEventInitMembers::default(),
    };
    let source = parsed.source.unwrap_or_else(|| v8::null(scope).into());
    if !source.is_null() {
        let Ok(object) = v8::Local::<v8::Object>::try_from(source) else {
            throw_type_error(
                scope,
                "Failed to construct 'CommandEvent': source must be an Element.",
            );
            return false;
        };
        if !event_init_value_is_element(scope, object) {
            throw_type_error(
                scope,
                "Failed to construct 'CommandEvent': source must be an Element.",
            );
            return false;
        }
    }
    let Some(command) = v8_string(scope, &parsed.command) else {
        return false;
    };
    set_private_value(scope, event, COMMAND_EVENT_SOURCE_SLOT, source);
    set_private_value(scope, event, COMMAND_EVENT_COMMAND_SLOT, command.into());
    true
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_track_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let track = init_value_property(scope, init, "track").unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, event, TRACK_EVENT_TRACK_SLOT, track);
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_interest_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) {
    let source =
        init_value_property(scope, init, "source").unwrap_or_else(|| v8::null(scope).into());
    let _ = InterestEventInitDeclaration::new(source).initialize(scope, event);
}

fn submit_event_submitter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(value) = init_value_property(scope, init, "submitter") else {
        return Some(v8::null(scope).into());
    };
    if value.is_null() {
        return Some(v8::null(scope).into());
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "Failed to construct 'SubmitEvent': submitter must be an HTMLElement.",
        );
        return None;
    };
    if !event_init_value_is_element(scope, object) {
        throw_type_error(
            scope,
            "Failed to construct 'SubmitEvent': submitter must be an HTMLElement.",
        );
        return None;
    }
    Some(value)
}

fn event_init_value_is_element(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let document_handle = unsafe { &*runtime_ptr }.dom_host().document_handle();
    let Some(handle) = crate::native_bridge::node_or_foreign_arg_handle_allow_detached(
        scope,
        runtime_ptr,
        Some(document_handle),
        object.into(),
    ) else {
        return false;
    };
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_element())
}
