use super::*;
use crate::context_bootstrap::events::EventInit;
use crate::context_bootstrap::history_mutation::document_can_have_url_rewritten;
use crate::context_bootstrap::location_runtime::location_href_slot;
use crate::context_bootstrap::navigation_activation::{
    NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT, install_navigation_transition,
};
use crate::context_bootstrap::navigation_events::{
    NAVIGATE_EVENT_PRECOMMIT_CONTROLLER_SLOT, navigation_destination_url_value,
    navigation_scroll_event_is_active, set_navigation_destination_url,
};
use crate::context_bootstrap::navigation_handler_callbacks::{
    NAVIGATE_EVENT_ADDED_HANDLERS_SLOT, NAVIGATE_EVENT_DEFERRED_HANDLERS_SLOT,
    NAVIGATE_EVENT_PRECOMMIT_HANDLERS_SLOT, navigation_handler_array_is_empty,
    push_navigation_handler, run_navigation_handler_array,
};
use crate::context_bootstrap::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, navigation_document_base_url,
    navigation_document_is_live, runtime_window_is_global, runtime_window_owner,
    window_location_for_holder,
};
use crate::context_bootstrap::web_storage::StorageReference;
use crate::native_bridge::element::scroll_to_url_fragment_or_top;
use crate::native_bridge::throw_dom_exception;
use crate::util::context_host_ptr_from_global_bridge;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;
use url::Url;
use webidl::WebIdlDictionarySerialize;

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
const PRECOMMIT_CONTROLLER_EVENT_SLOT: &str = "__lmPrecommitControllerEvent";
const PRECOMMIT_CONTROLLER_BACKING_SLOT: &str = "__moliPrecommitControllerBacking";
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
#[webapi(prototype = "Object", interface = web_api_interfaces::NavigationPrecommitController)]
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
#[webapi(interface = web_api_interfaces::NavigateEvent)]
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

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "StorageEventInit", snapshot)]
pub(in crate::context_bootstrap::events) struct StorageEventInitMembers<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, converter = "raw")]
    key: Option<webidl::DomString16>,
    #[webidl(name = "newValue", nullable, converter = "raw")]
    new_value: Option<webidl::DomString16>,
    #[webidl(name = "oldValue", nullable, converter = "raw")]
    old_value: Option<webidl::DomString16>,
    #[webidl(name = "storageArea", converter = "raw", nullable)]
    storage_area: Option<StorageReference<'s>>,
    #[webidl(default = "", converter = "usv_string")]
    url: String,
}

impl<'s> StorageEventInitMembers<'s> {
    pub(in crate::context_bootstrap::events) fn from_native(
        key: Option<&[u16]>,
        old_value: Option<&[u16]>,
        new_value: Option<&[u16]>,
        url: &str,
        storage_area: Option<v8::Local<'s, v8::Value>>,
    ) -> Self {
        Self {
            base: EventInit::default(),
            key: key.map(|value| webidl::DomString16(value.to_vec())),
            old_value: old_value.map(|value| webidl::DomString16(value.to_vec())),
            new_value: new_value.map(|value| webidl::DomString16(value.to_vec())),
            url: url.to_owned(),
            storage_area: storage_area
                .and_then(|value| value.try_into().ok())
                .map(StorageReference),
        }
    }
}

impl StorageEventInitMembers<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
}

pub(super) fn parse_storage_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<StorageEventInitMembers<'s>> {
    crate::context_bootstrap::events::parse_event_dictionary(scope, args, "StorageEvent")
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_storage_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    parsed: StorageEventInitMembers<'s>,
) {
    parsed
        .initialize_snapshot(scope, event)
        .expect("StorageEvent state");
}

pub(super) fn initialize_navigation_current_entry_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: super::navigation_init::NavigationCurrentEntryChangeEventInitMembers<'s>,
) {
    init.initialize_snapshot(scope, event)
        .expect("current entry event state");
}

pub(super) fn initialize_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: super::navigation_init::NavigateEventInitMembers<'s>,
) {
    init.initialize_snapshot(scope, event)
        .expect("NavigateEvent state");
    define_navigate_event_internal_flag(scope, event, true);
}

fn define_navigate_event_internal_flag(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    synthetic: bool,
) {
    crate::context_bootstrap::set_event_private_value(
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
    crate::context_bootstrap::event_private_value(scope, event, slot)
        .filter(|value| !value.is_undefined())
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
    crate::context_bootstrap::set_event_private_value(
        scope,
        event,
        slot,
        v8::Boolean::new(scope, value).into(),
    );
}

fn navigate_event_can_use_navigation_api<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    let synthetic = navigate_event_private_bool(scope, event, NAVIGATE_EVENT_SYNTHETIC_SLOT, true);
    let can_intercept =
        crate::context_bootstrap::event_bool_attribute(scope, event, "canIntercept");
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
    let Some(target) = crate::context_bootstrap::event_backing(scope, event)
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
    if crate::context_bootstrap::event_bool_attribute(scope, event, "defaultPrevented") {
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
    let view = event;
    let event = crate::context_bootstrap::events::event_backing(scope, view);
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
        && !crate::context_bootstrap::event_bool_attribute(scope, event, "cancelable")
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
            view,
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
            push_navigation_handler(scope, view, NAVIGATE_EVENT_DEFERRED_HANDLERS_SLOT, handler);
    }
}

/// Runs precommit handlers only after the complete `navigate` event dispatch.
///
/// The navigation transaction retires the controller when interception commits.
/// Aborting a precommit navigation prevents its commit; it does not change the
/// old event's interception state or disable edits to that event.
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
    crate::context_bootstrap::set_event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_CONTROLLER_SLOT,
        controller.into(),
    );
    let arguments = [controller.into()];
    run_navigation_handler_array(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_HANDLERS_SLOT,
        &arguments,
    )
}

fn install_navigate_event_precommit_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    if crate::context_bootstrap::event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT,
    )
    .is_some_and(|value| !value.is_undefined())
    {
        return;
    }
    let Some(navigation) = crate::context_bootstrap::event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_NAVIGATION_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()) else {
        return;
    };
    let Some(from) = crate::context_bootstrap::event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_FROM_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()) else {
        return;
    };
    let destination = crate::context_bootstrap::event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_DESTINATION_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let navigation_type = crate::context_bootstrap::event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_TYPE_SLOT,
    )
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
    crate::context_bootstrap::set_event_private_value(
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

fn precommit_controller_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let controller = v8::Local::<v8::Object>::try_from(data).ok()?;
    let backing =
        crate::util::get_private_object(scope, controller, PRECOMMIT_CONTROLLER_BACKING_SLOT)
            .unwrap_or(controller);
    if !get_private_value(scope, backing, PRECOMMIT_CONTROLLER_ACTIVE_SLOT)
        .is_some_and(|value| value.is_true())
    {
        navigate_event_throw_invalid_state(scope);
        return None;
    }
    let event = get_private_value(scope, controller, PRECOMMIT_CONTROLLER_EVENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let owner_is_active = crate::context_bootstrap::event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_NAVIGATION_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    .is_some_and(|navigation| {
        let owner = runtime_window_owner(scope, navigation);
        navigation_document_is_live(scope, owner)
    });
    if !owner_is_active
        || !navigate_event_target_is_connected(scope, event)
        || crate::context_bootstrap::event_bool_attribute(scope, event, "defaultPrevented")
    {
        navigate_event_throw_invalid_state(scope);
        return None;
    }
    Some(event)
}

pub(in crate::context_bootstrap) fn navigation_precommit_controller_for_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    argument: v8::Local<'s, v8::Value>,
    view: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let Ok(controller) = v8::Local::<v8::Object>::try_from(argument) else {
        return argument;
    };
    if get_private_value(scope, controller, PRECOMMIT_CONTROLLER_EVENT_SLOT).is_none() {
        return argument;
    }
    let Some(context) = view.get_creation_context(scope) else {
        return argument;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let wrapper = create_precommit_controller(scope, view);
    set_private_value(
        scope,
        wrapper,
        PRECOMMIT_CONTROLLER_BACKING_SLOT,
        controller.into(),
    );
    wrapper.into()
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
    let navigation_type = crate::context_bootstrap::event_backing(scope, event)
        .get(scope, v8str(scope, "navigationType").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if matches!(navigation_type.as_str(), "reload" | "traverse") {
        navigate_event_throw_invalid_state(scope);
        return;
    }
    let Some(destination) = crate::context_bootstrap::event_backing(scope, event)
        .get(scope, v8str(scope, "destination").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        navigate_event_throw_invalid_state(scope);
        return;
    };
    let Some(navigation) = crate::context_bootstrap::event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_NAVIGATION_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()) else {
        navigate_event_throw_invalid_state(scope);
        return;
    };
    let owner = runtime_window_owner(scope, navigation);
    let current_href = window_location_for_holder(scope, owner)
        .and_then(|location| location_href_slot(scope, location))
        .unwrap_or_default();
    let Some(raw_url) = args.get(0).to_string(scope) else {
        return;
    };
    let raw_url = raw_url.to_rust_string_lossy(scope);
    let Some(base_url) = navigation_document_base_url(scope, owner, &current_href) else {
        navigate_event_throw_invalid_state(scope);
        return;
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
    if !Url::parse(&current_href)
        .is_ok_and(|document_url| document_can_have_url_rewritten(&document_url, &redirected))
    {
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
    set_navigation_destination_url(scope, destination, redirected.as_str());
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
        let history = v8_string(scope, &history).unwrap();
        crate::context_bootstrap::set_event_private_value(
            scope,
            event,
            NAVIGATE_EVENT_REDIRECT_HISTORY_SLOT,
            history.into(),
        );
        define_event_property(scope, event, "navigationType", history.into());
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
    let default_prevented =
        crate::context_bootstrap::event_bool_attribute(scope, event, "defaultPrevented");
    let already_scrolled =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_SCROLL_CALLED_SLOT, false);
    let active = crate::context_bootstrap::event_backing(scope, event)
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
    let Some(target_url) = crate::context_bootstrap::event_backing(scope, event)
        .get(scope, v8str(scope, "destination").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|destination| navigation_destination_url_value(scope, destination))
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

pub(in crate::context_bootstrap::events) fn initialize_navigate_event_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    NavigateEventMethodsDeclaration::default()
        .initialize(scope, event)
        .expect("NavigateEvent methods declaration should initialize");
}


#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ClipboardChangeEventInit")]
pub(super) struct ClipboardChangeEventInitMembers<'scope> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "changeId", with = clipboard_change_event_change_id_member)]
    change_id: v8::Local<'scope, v8::BigInt>,
    #[webidl(with = clipboard_change_event_types_member)]
    types: Vec<v8::Local<'scope, v8::String>>,
}

struct ClipboardChangeEventType<'scope>(v8::Local<'scope, v8::String>);

impl<'scope> webidl::WebIdlConverter<'scope> for ClipboardChangeEventType<'scope> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'scope, '_>,
        value: v8::Local<'scope, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        value
            .to_string(scope)
            .map(Self)
            .ok_or_else(|| webidl::WebIdlError::pending_exception(context))
    }
}

impl ClipboardChangeEventInitMembers<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
}

fn clipboard_change_event_change_id_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::BigInt>, webidl::WebIdlError> {
    let context = webidl::Context::member("ClipboardChangeEventInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(v8::BigInt::new_from_u64(scope, 0));
    };
    if value.is_undefined() {
        return Ok(v8::BigInt::new_from_u64(scope, 0));
    }
    value
        .to_big_int(scope)
        .ok_or_else(|| webidl::WebIdlError::pending_exception(context))
}

fn clipboard_change_event_types_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Vec<v8::Local<'s, v8::String>>, webidl::WebIdlError> {
    let context = webidl::Context::member("ClipboardChangeEventInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(Vec::new());
    };
    if value.is_undefined() {
        return Ok(Vec::new());
    }
    webidl::convert::<webidl::Sequence<ClipboardChangeEventType<'s>>>(scope, value, context)
        .map(|sequence| sequence.0.into_iter().map(|value| value.0).collect())
}

pub(super) fn parse_clipboard_change_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ClipboardChangeEventInitMembers<'s>> {
    let context = webidl::Context::argument("ClipboardChangeEvent", 2);
    let object = match webidl::dictionary_arg(args, 1, context) {
        Ok(object) => object,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Some(object) = object else {
        return Some(ClipboardChangeEventInitMembers {
            base: EventInit::default(),
            change_id: v8::BigInt::new_from_u64(scope, 0),
            types: Vec::new(),
        });
    };
    match webidl::parse_dictionary_object(scope, object) {
        Ok(init) => Some(init),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(in crate::context_bootstrap::events::subclasses) fn initialize_clipboard_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: ClipboardChangeEventInitMembers<'s>,
) -> bool {
    let mut elements = Vec::with_capacity(init.types.len());
    for event_type in init.types {
        elements.push(event_type.into());
    }
    let types = v8::Array::new_with_elements(scope, &elements);
    if types.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return false;
    }
    set_private_value(
        scope,
        event,
        CLIPBOARD_CHANGE_EVENT_TYPES_SLOT,
        types.into(),
    );
    set_private_value(
        scope,
        event,
        CLIPBOARD_CHANGE_EVENT_CHANGE_ID_SLOT,
        init.change_id.into(),
    );
    true
}
