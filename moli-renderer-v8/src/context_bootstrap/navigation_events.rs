use super::events::{
    construct_original_before_unload_event, construct_original_event,
    construct_original_page_transition_event, run_navigate_event_precommit_handlers,
};
use super::location_history_storage::{
    NAVIGATION_ENTRY_EVENT_LISTENERS_SLOT, NAVIGATION_EVENT_LISTENERS_SLOT,
};
use super::location_runtime::location_href_slot;
use super::media_queries::dispatch_simple_event_target_event;
use super::navigation_activation::{
    navigation_transition_matches_resolver, precommit_transition_resolver_from_event,
    take_navigation_transition_committed_resolver,
};
use super::navigation_entry::{
    history_entries, navigation_current_entry, navigation_entries_share_document,
    navigation_entries_share_origin, navigation_entry_id_value, navigation_entry_index_value,
    navigation_entry_key_value, navigation_entry_url_value,
    save_current_navigation_entry_scroll_position,
};
use super::navigation_entry_state::clone_navigation_entry_state;
use super::navigation_handler_callbacks::{
    NAVIGATE_EVENT_ADDED_HANDLERS_SLOT, NAVIGATE_EVENT_DEFERRED_HANDLERS_SLOT,
    run_navigation_handler_arrays,
};
use super::navigation_lifecycle::{
    finish_navigation_error_events, settle_navigation_transition_finished_local,
};
use super::navigation_result::navigation_dom_exception;
use super::navigation_window::{
    child_browsing_context_handle_for_runtime_owner, navigation_document_is_active,
    navigation_has_disabled_entries, replace_navigation_unload_event_active,
    runtime_window_dispatch_scope, runtime_window_is_global, runtime_window_owner,
    should_dispatch_hash_change, window_history_for_holder, window_location_for_holder,
    window_task_target_for_runtime_owner,
};
use super::*;
use crate::document_runtime::EventTargetHandle;
use crate::page_task_queue::RendererPageHashChangeData;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const NAVIGATION_DESTINATION_STATE_SLOT: &str = "__lmNavigationDestinationState";
const NAVIGATION_DESTINATION_ENTRY_SLOT: &str = "__lmNavigationDestinationEntry";
const NAVIGATION_DESTINATION_BACKING_SLOT: &str = "__moliNavigationDestinationBacking";
const NAVIGATION_DESTINATION_URL_SLOT: &str = "__lmNavigationDestinationUrl";
const NAVIGATION_DESTINATION_SAME_DOCUMENT_SLOT: &str = "__lmNavigationDestinationSameDocument";
const NAVIGATE_EVENT_SYNTHETIC_SLOT: &str = "__lmNavigateEventSynthetic";
const NAVIGATE_EVENT_INTERCEPTED_SLOT: &str = "__lmNavigateEventIntercepted";
const NAVIGATION_ERROR_EVENT_ACTIVE_SLOT: &str = "__lmNavigationErrorEventActive";
const NAVIGATE_EVENT_INTERCEPT_ERROR_SLOT: &str = "__lmNavigateEventInterceptError";
const NAVIGATE_EVENT_INTERCEPT_RESULT_SLOT: &str = "__lmNavigateEventInterceptResult";
const NAVIGATE_EVENT_PRECOMMIT_SEEN_SLOT: &str = "__lmNavigateEventPrecommitSeen";
pub(super) const NAVIGATE_EVENT_PRECOMMIT_CONTROLLER_SLOT: &str =
    "__lmNavigateEventPrecommitController";
const NAVIGATE_EVENT_REDIRECTED_SLOT: &str = "__lmNavigateEventRedirected";
const NAVIGATE_EVENT_REDIRECT_HISTORY_SLOT: &str = "__lmNavigateEventRedirectHistory";
const NAVIGATE_EVENT_ABORT_ERROR_SLOT: &str = "__lmNavigateEventAbortError";
const NAVIGATE_EVENT_FOCUS_RESET_SLOT: &str = "__lmNavigateEventFocusReset";
pub(super) const NAVIGATE_EVENT_SCROLL_AFTER_TRANSITION_SLOT: &str =
    "__lmNavigateEventScrollAfterTransition";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_NAVIGATION_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionNavigation";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_FROM_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionFrom";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_DESTINATION_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionDestination";
const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_TYPE_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionType";
pub(super) const NAVIGATE_EVENT_SCROLL_CALLED_SLOT: &str = "__lmNavigateEventScrollCalled";
const NAVIGATION_ACTIVE_NAVIGATE_EVENT_SLOT: &str = "__lmNavigationActiveNavigateEvent";
pub(super) const NAVIGATION_FOCUS_RESET_EPOCH_SLOT: &str = "__lmNavigationFocusResetEpoch";
pub(in crate::context_bootstrap) const NAVIGATION_ACTIVE_SCROLL_EVENT_SLOT: &str =
    "__lmNavigationActiveScrollEvent";
pub(super) const NAVIGATION_SCROLL_TARGET_HREF_SLOT: &str = "__lmNavigationScrollTargetHref";
const ACTIVE_NAVIGATE_EVENT_EVENT_SLOT: &str = "__lmActiveNavigateEventEvent";
const ACTIVE_NAVIGATE_EVENT_SIGNAL_SLOT: &str = "__lmActiveNavigateEventSignal";
const ACTIVE_NAVIGATE_EVENT_HREF_SLOT: &str = "__lmActiveNavigateEventHref";

#[derive(WebApiObject)]
#[webapi(plain)]
struct NavigationCurrentEntryChangeEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    from: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    navigation_type: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NavigationErrorEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    message: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    filename: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct PopStateEventStateDeclaration<'scope> {
    #[webapi(data_property)]
    state: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct HashChangeEventStateDeclaration {
    #[webapi(data_property = "oldURL")]
    old_url: String,
    #[webapi(data_property = "newURL")]
    new_url: String,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NavigateEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    navigation_type: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    destination: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    can_intercept: bool,
    #[webapi(data_property, enumerable)]
    user_initiated: bool,
    #[webapi(data_property, enumerable)]
    hash_change: bool,
    #[webapi(data_property, enumerable)]
    signal: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    form_data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    download_request: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    info: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    source_element: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    cancelable: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ActiveNavigateEventDataDeclaration<'scope> {
    #[webapi(slot = ACTIVE_NAVIGATE_EVENT_EVENT_SLOT)]
    event: v8::Local<'scope, v8::Object>,
    #[webapi(slot = ACTIVE_NAVIGATE_EVENT_SIGNAL_SLOT)]
    signal: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = ACTIVE_NAVIGATE_EVENT_HREF_SLOT)]
    href: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigationDestination)]
struct NavigationDestinationDeclaration<'scope> {
    #[webapi(slot = NAVIGATION_DESTINATION_URL_SLOT)]
    url: v8::Local<'scope, v8::String>,
    #[webapi(slot = NAVIGATION_DESTINATION_SAME_DOCUMENT_SLOT)]
    same_document: bool,
    #[webapi(slot = NAVIGATION_DESTINATION_ENTRY_SLOT)]
    entry: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = NAVIGATION_DESTINATION_STATE_SLOT)]
    state: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NavigationDestination, enumerable, receiver)]
struct NavigationDestinationPrototypeDeclaration {
    #[webapi(accessor_property, getter = navigation_destination_url_getter)]
    url: (),
    #[webapi(accessor_property, getter = navigation_destination_key_getter)]
    key: (),
    #[webapi(accessor_property, getter = navigation_destination_id_getter)]
    id: (),
    #[webapi(accessor_property, getter = navigation_destination_index_getter)]
    index: (),
    #[webapi(accessor_property, getter = navigation_destination_same_document_getter)]
    same_document: (),
    #[webapi(method, length = 0, callback = navigation_destination_get_state_callback)]
    get_state: (),
}

pub(super) fn install_navigation_destination_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    NavigationDestinationPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(super) struct NavigationDispatchOutcome<'s> {
    pub(super) proceed: bool,
    pub(super) intercepted: bool,
    pub(super) signal: Option<v8::Local<'s, v8::Object>>,
    pub(super) destination: Option<v8::Local<'s, v8::Object>>,
    pub(super) redirected_url: Option<String>,
    pub(super) redirected_history: Option<String>,
    pub(super) redirected_state: Option<v8::Local<'s, v8::Value>>,
    pub(super) precommit_event: Option<v8::Local<'s, v8::Object>>,
    pub(super) precommit_error: Option<v8::Local<'s, v8::Value>>,
    pub(super) precommit_result: Option<v8::Local<'s, v8::Value>>,
    pub(super) intercept_error: Option<v8::Local<'s, v8::Value>>,
    pub(super) intercept_result: Option<v8::Local<'s, v8::Value>>,
    pub(super) abort_error: Option<v8::Local<'s, v8::Value>>,
}

impl<'s> NavigationDispatchOutcome<'s> {
    pub(super) fn proceed() -> Self {
        Self {
            proceed: true,
            intercepted: false,
            signal: None,
            destination: None,
            redirected_url: None,
            redirected_history: None,
            redirected_state: None,
            precommit_event: None,
            precommit_error: None,
            precommit_result: None,
            intercept_error: None,
            intercept_result: None,
            abort_error: None,
        }
    }
}

pub(super) fn mark_navigation_outcome_default_prevented<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    outcome: &NavigationDispatchOutcome<'s>,
) {
    let Some(event) = outcome.precommit_event else {
        return;
    };
    let _ = crate::context_bootstrap::event_backing(scope, event).define_own_property(
        scope,
        v8str(scope, "defaultPrevented").into(),
        v8::Boolean::new(scope, true).into(),
        Default::default(),
    );
}

fn set_navigation_focus_reset_epoch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    epoch: Option<u64>,
) {
    match epoch {
        Some(epoch) => {
            let value = v8::BigInt::new_from_u64(scope, epoch);
            set_private_value(
                scope,
                navigation,
                NAVIGATION_FOCUS_RESET_EPOCH_SLOT,
                value.into(),
            );
        }
        None => {
            set_private_value(
                scope,
                navigation,
                NAVIGATION_FOCUS_RESET_EPOCH_SLOT,
                v8::undefined(scope).into(),
            );
        }
    }
}

pub(super) fn navigation_active_scroll_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, navigation, NAVIGATION_ACTIVE_SCROLL_EVENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn navigation_scroll_target_href<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, navigation, NAVIGATION_SCROLL_TARGET_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn navigation_focus_reset_epoch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let value = get_private_value(scope, navigation, NAVIGATION_FOCUS_RESET_EPOCH_SLOT)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (epoch, lossless) = value.u64_value();
    lossless.then_some(epoch)
}

pub(super) fn navigation_error_event_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, navigation, NAVIGATION_ERROR_EVENT_ACTIVE_SLOT)
        .is_some_and(|value| value.is_boolean() && value.boolean_value(scope))
}

fn set_navigation_error_event_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    active: bool,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ERROR_EVENT_ACTIVE_SLOT,
        v8::Boolean::new(scope, active).into(),
    );
}

pub(super) fn clear_navigation_focus_reset_epoch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_FOCUS_RESET_EPOCH_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(super) fn clear_navigation_scroll_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_SCROLL_EVENT_SLOT,
        v8::undefined(scope).into(),
    );
    set_private_value(
        scope,
        navigation,
        NAVIGATION_SCROLL_TARGET_HREF_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(super) fn navigation_has_active_scroll_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, navigation, NAVIGATION_ACTIVE_SCROLL_EVENT_SLOT)
        .is_some_and(|value| !value.is_undefined())
}

pub(in crate::context_bootstrap) fn navigation_scroll_event_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, navigation, NAVIGATION_ACTIVE_SCROLL_EVENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .is_some_and(|active| {
            let active = event_backing(scope, active);
            active.strict_equals(event_backing(scope, event).into())
        })
}

fn navigation_active_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, navigation, NAVIGATION_ACTIVE_NAVIGATE_EVENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_navigation_active_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_NAVIGATE_EVENT_SLOT,
        data.into(),
    );
}

fn create_active_navigate_event_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    signal: Option<v8::Local<'s, v8::Object>>,
    href: &str,
) -> v8::Local<'s, v8::Object> {
    let href = v8_string(scope, href)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::String::empty(scope).into());
    ActiveNavigateEventDataDeclaration::new(event, signal, href)
        .bind(scope)
        .expect("active NavigateEvent data declaration should bind")
}

fn clear_navigation_active_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_NAVIGATE_EVENT_SLOT,
        v8::undefined(scope).into(),
    );
}

fn clear_navigation_active_navigate_event_if_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Object>,
) {
    if navigation_active_navigate_event(scope, navigation)
        .is_some_and(|active| active.strict_equals(data.into()))
    {
        clear_navigation_active_navigate_event(scope, navigation);
    }
}

fn set_navigation_scroll_target_href<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    href: Option<&str>,
) {
    match href {
        Some(href) => {
            let value = v8_string(scope, href)
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::String::empty(scope).into());
            set_private_value(scope, navigation, NAVIGATION_SCROLL_TARGET_HREF_SLOT, value);
        }
        None => {
            set_private_value(
                scope,
                navigation,
                NAVIGATION_SCROLL_TARGET_HREF_SLOT,
                v8::undefined(scope).into(),
            );
        }
    }
}

fn set_navigation_scroll_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    href: &str,
    intercepted: bool,
) {
    if !intercepted {
        clear_navigation_scroll_state(scope, navigation);
        return;
    }
    set_private_value(
        scope,
        navigation,
        NAVIGATION_ACTIVE_SCROLL_EVENT_SLOT,
        event.into(),
    );
    if navigate_event_private_bool(
        scope,
        event,
        NAVIGATE_EVENT_SCROLL_AFTER_TRANSITION_SLOT,
        true,
    ) {
        set_navigation_scroll_target_href(scope, navigation, Some(href));
    } else {
        set_navigation_scroll_target_href(scope, navigation, None);
    }
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

fn install_precommit_transition_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    destination: v8::Local<'s, v8::Object>,
    navigation_type: &str,
) {
    let owner = runtime_window_owner(scope, navigation);
    let Some(from) = navigation_current_entry(scope, owner) else {
        return;
    };
    crate::context_bootstrap::set_event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_NAVIGATION_SLOT,
        navigation.into(),
    );
    crate::context_bootstrap::set_event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_FROM_SLOT,
        from.into(),
    );
    crate::context_bootstrap::set_event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_DESTINATION_SLOT,
        destination.into(),
    );
    let navigation_type = v8_string(scope, navigation_type)
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::String::empty(scope).into());
    crate::context_bootstrap::set_event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_TYPE_SLOT,
        navigation_type,
    );
}

pub(super) fn cancel_active_navigation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let data = navigation_active_navigate_event(scope, navigation)?;
    let event = get_private_value(scope, data, ACTIVE_NAVIGATE_EVENT_EVENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let signal = get_private_value(scope, data, ACTIVE_NAVIGATE_EVENT_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let href = get_private_value(scope, data, ACTIVE_NAVIGATE_EVENT_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    clear_navigation_active_navigate_event(scope, navigation);
    // Aborting a navigation cancels an event that is still being dispatched,
    // including iframe traversals whose events are not author-cancelable.
    // A precommit handler runs after dispatch, so aborting there must leave
    // the event's canceled flag unchanged.
    if super::events::event_is_dispatching(scope, event) {
        let _ = event_backing(scope, event).define_own_property(
            scope,
            v8str(scope, "defaultPrevented").into(),
            v8::Boolean::new(scope, true).into(),
            Default::default(),
        );
    }
    let error = navigation_dom_exception(scope, "Navigation was canceled", "AbortError");
    set_event_private_value(scope, event, NAVIGATE_EVENT_ABORT_ERROR_SLOT, error);
    let transition_resolver = precommit_transition_resolver_from_event(scope, event);
    // Abort/error listeners can start another navigation before this cancellation
    // returns. Retain the old committed resolver before that navigation commits.
    let committed_resolver = transition_resolver
        .filter(|resolver| navigation_transition_matches_resolver(scope, navigation, *resolver))
        .and_then(|_| take_navigation_transition_committed_resolver(scope, navigation));
    if let Some(signal) = signal {
        crate::native_bridge::abort::abort_signal(scope, signal, error);
    }
    finish_navigation_error_events(scope, navigation, error, &href);
    if let Some(resolver) = committed_resolver {
        let _ = resolver.reject(scope, error);
    }
    settle_navigation_transition_finished_local(
        scope,
        navigation,
        transition_resolver,
        Some(error),
    );
    Some(error)
}

pub(super) fn dispatch_popstate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    owner: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    let context = owner
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let Ok(event_ctor) =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "PopStateEvent")
    else {
        return;
    };
    let init = PopStateEventStateDeclaration::new(state)
        .bind(scope)
        .expect("PopStateEvent init declaration should bind");
    let Some(event) =
        event_ctor.new_instance(scope, &[v8str(scope, "popstate").into(), init.into()])
    else {
        return;
    };
    mark_event_trusted(scope, event);
    let runtime = unsafe { &mut *host_ptr };
    match runtime_window_dispatch_scope(scope, owner) {
        Some(crate::native_bridge::OwnerDispatchScope::Child(handle)) => {
            runtime.dispatch_child_window_event(scope, handle, "popstate", event);
        }
        Some(crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id)) => {
            runtime.dispatch_lightweight_popup_window_event(scope, popup_id, "popstate", event);
        }
        Some(crate::native_bridge::OwnerDispatchScope::Top) => {
            let _ = runtime.dispatch_public_event_best_effort(
                scope,
                host_ptr,
                EventTargetHandle::Window,
                event,
                "window popstate event",
            );
        }
        None => {}
    }
}

pub(crate) fn construct_original_hash_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    old_url: &str,
    new_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let event_ctor =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "HashChangeEvent")
            .ok()?;
    let init = HashChangeEventStateDeclaration::new(old_url.to_owned(), new_url.to_owned())
        .bind(scope)
        .expect("HashChangeEvent init declaration should bind");
    let event =
        event_ctor.new_instance(scope, &[v8str(scope, "hashchange").into(), init.into()])?;
    mark_event_trusted(scope, event);
    Some(event)
}

pub(crate) fn dispatch_beforeunload_for_runtime_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    let Some(event) = construct_original_before_unload_event(scope) else {
        return;
    };
    dispatch_unload_lifecycle_event_for_runtime_owner(scope, owner, "beforeunload", event);
}

pub(crate) fn dispatch_unload_for_runtime_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    let Some(event) = construct_original_event(scope, "unload") else {
        return;
    };
    dispatch_unload_lifecycle_event_for_runtime_owner(scope, owner, "unload", event);
}

pub(crate) fn dispatch_pagehide_for_runtime_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    let Some(event) = construct_original_page_transition_event(scope, "pagehide", false) else {
        return;
    };
    dispatch_unload_lifecycle_event_for_runtime_owner(scope, owner, "pagehide", event);
}

fn dispatch_unload_lifecycle_event_for_runtime_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
) {
    let _document_unload = context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
        let host = unsafe { &*host_ptr };
        super::window_accessors::window_document_handle(scope, owner, host)
            .map(|document| host.enter_document_unload(document))
    });
    let previous_unload = replace_navigation_unload_event_active(scope, owner, true);
    if runtime_window_is_global(scope, owner) {
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let host = unsafe { &mut *host_ptr };
            let _ = host.dispatch_public_event_best_effort(
                scope,
                host_ptr,
                EventTargetHandle::Window,
                event,
                "window unload lifecycle event",
            );
        }
    } else if let Some(child_handle) = child_browsing_context_handle_for_runtime_owner(scope, owner)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.dispatch_child_window_event_with_target_override(
            scope,
            child_handle,
            event_type,
            event,
            matches!(event_type, "pagehide" | "unload"),
        );
    } else if let Some(popup_id) =
        crate::native_bridge::lightweight_popup_id_from_window(scope, owner)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }
            .dispatch_lightweight_popup_window_event(scope, popup_id, event_type, event);
    }
    replace_navigation_unload_event_active(scope, owner, previous_unload);
}

pub(super) fn queue_hash_change_for_runtime_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    old_url: Option<&str>,
    new_url: &str,
) {
    let Some(old_url) = old_url else {
        return;
    };
    if !should_dispatch_hash_change(old_url, new_url) {
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(target) = window_task_target_for_runtime_owner(scope, host, owner) else {
        return;
    };
    let sender = host.page_hash_change_delivery_sender();
    if sender
        .send(
            target,
            RendererPageHashChangeData::new(old_url.to_owned(), new_url.to_owned()),
        )
        .is_err()
    {
        tracing::debug!(
            ?target,
            "retired Page DOM-manipulation route rejected hashchange delivery"
        );
    }
}

pub(super) fn dispatch_navigation_currententrychange<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    from: Option<v8::Local<'s, v8::Object>>,
    navigation_type: Option<&str>,
) {
    if navigation_has_disabled_entries(scope, navigation) {
        return;
    }
    let context = navigation
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let Ok(event_ctor) = super::exposed_interfaces::ensure_intrinsic_interface_constructor(
        scope,
        "NavigationCurrentEntryChangeEvent",
    ) else {
        return;
    };
    let init = NavigationCurrentEntryChangeEventInitDeclaration {
        from: from
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        navigation_type: navigation_type
            .and_then(|value| v8_string(scope, value))
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
    }
    .bind(scope)
    .expect("NavigationCurrentEntryChangeEvent init declaration should bind");
    let Some(event) = event_ctor.new_instance(
        scope,
        &[v8str(scope, "currententrychange").into(), init.into()],
    ) else {
        return;
    };
    mark_event_trusted(scope, event);
    let _ = dispatch_simple_event_target_event(
        scope,
        navigation,
        NAVIGATION_EVENT_LISTENERS_SLOT,
        "currententrychange",
        event,
    );
}

pub(super) fn dispatch_navigation_entry_dispose<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) {
    let context = entry
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(event) = construct_original_event(scope, "dispose") else {
        return;
    };
    let _ = dispatch_simple_event_target_event(
        scope,
        entry,
        NAVIGATION_ENTRY_EVENT_LISTENERS_SLOT,
        "dispose",
        event,
    );
}

pub(super) fn dispatch_navigation_success<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    let context = navigation
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(event) = construct_original_event(scope, "navigatesuccess") else {
        return;
    };
    let _ = dispatch_simple_event_target_event(
        scope,
        navigation,
        NAVIGATION_EVENT_LISTENERS_SLOT,
        "navigatesuccess",
        event,
    );
}

pub(super) fn dispatch_navigation_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
    filename: &str,
) {
    let context = navigation
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let Ok(event_ctor) =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "ErrorEvent")
    else {
        return;
    };
    let message = error
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let init = NavigationErrorEventInitDeclaration {
        message: v8_string(scope, &message).unwrap_or_else(|| v8::String::empty(scope)),
        filename: v8_string(scope, filename).unwrap_or_else(|| v8::String::empty(scope)),
        lineno: 1,
        colno: 1,
        error,
    }
    .bind(scope)
    .expect("NavigationErrorEvent init declaration should bind");
    let event_type = v8str(scope, "navigateerror");
    let event = event_ctor.new_instance(scope, &[event_type.into(), init.into()]);
    let Some(event) = event else {
        return;
    };
    mark_event_trusted(scope, event);
    set_navigation_error_event_active(scope, navigation, true);
    let _ = dispatch_simple_event_target_event(
        scope,
        navigation,
        NAVIGATION_EVENT_LISTENERS_SLOT,
        "navigateerror",
        event,
    );
    set_navigation_error_event_active(scope, navigation, false);
}

pub(super) fn dispatch_navigation_navigate_event_with_outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    href: &str,
    navigation_type: &str,
    hash_change: bool,
    same_document: bool,
    can_intercept: bool,
    user_initiated: bool,
    download_request: Option<&str>,
    state: Option<v8::Local<'s, v8::Value>>,
    info: Option<v8::Local<'s, v8::Value>>,
    source_element: Option<v8::Local<'s, v8::Object>>,
) -> NavigationDispatchOutcome<'s> {
    dispatch_navigation_navigate_event_with_form_data_and_outcome(
        scope,
        navigation,
        href,
        navigation_type,
        hash_change,
        same_document,
        can_intercept,
        user_initiated,
        download_request,
        state,
        info,
        None,
        source_element,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_navigation_navigate_event_with_form_data_and_outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    href: &str,
    navigation_type: &str,
    hash_change: bool,
    same_document: bool,
    can_intercept: bool,
    user_initiated: bool,
    download_request: Option<&str>,
    state: Option<v8::Local<'s, v8::Value>>,
    info: Option<v8::Local<'s, v8::Value>>,
    form_data: Option<v8::Local<'s, v8::Value>>,
    source_element: Option<v8::Local<'s, v8::Object>>,
) -> NavigationDispatchOutcome<'s> {
    if navigation_has_disabled_entries(scope, navigation) {
        return NavigationDispatchOutcome::proceed();
    }
    let context = navigation
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let focus_reset_epoch = context_host_ptr_from_global_bridge(scope)
        .map(|host_ptr| unsafe { &*host_ptr }.focus_change_epoch());
    let Ok(event_ctor) =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "NavigateEvent")
    else {
        return NavigationDispatchOutcome::proceed();
    };
    let destination = create_navigation_destination(scope, href, same_document, state);
    let signal = create_navigation_abort_signal(scope);
    let signal_object = v8::Local::<v8::Object>::try_from(signal).ok();
    let init = NavigateEventInitDeclaration {
        navigation_type: v8_string(scope, navigation_type)
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        destination,
        can_intercept,
        user_initiated,
        hash_change,
        signal,
        form_data: form_data.unwrap_or_else(|| v8::null(scope).into()),
        download_request: download_request
            .and_then(|value| v8_string(scope, value))
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        info: info.unwrap_or_else(|| v8::undefined(scope).into()),
        source_element: source_element
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        cancelable: true,
    }
    .bind(scope)
    .expect("NavigateEvent init declaration should bind");
    let Some(event) =
        event_ctor.new_instance(scope, &[v8str(scope, "navigate").into(), init.into()])
    else {
        return NavigationDispatchOutcome::proceed();
    };
    mark_event_trusted(scope, event);
    set_navigate_event_private_bool(scope, event, NAVIGATE_EVENT_SYNTHETIC_SLOT, false);
    set_navigate_event_private_bool(scope, event, NAVIGATE_EVENT_FOCUS_RESET_SLOT, true);
    set_navigate_event_private_bool(
        scope,
        event,
        NAVIGATE_EVENT_SCROLL_AFTER_TRANSITION_SLOT,
        true,
    );
    install_precommit_transition_seed(scope, navigation, event, destination, navigation_type);
    let active_data = create_active_navigate_event_data(scope, event, signal_object, href);
    set_navigation_active_navigate_event(scope, navigation, active_data);
    let owner = runtime_window_owner(scope, navigation);
    save_current_navigation_entry_scroll_position(scope, owner);
    let proceed = dispatch_simple_event_target_event(
        scope,
        navigation,
        NAVIGATION_EVENT_LISTENERS_SLOT,
        "navigate",
        event,
    );
    let (precommit_error, precommit_result) = if proceed {
        run_navigate_event_precommit_handlers(scope, event)
    } else {
        (None, None)
    };
    clear_navigation_active_navigate_event_if_current(scope, navigation, active_data);
    let precommit_seen =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_PRECOMMIT_SEEN_SLOT, false);
    let intercepted =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_INTERCEPTED_SLOT, false);
    let focus_reset =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_FOCUS_RESET_SLOT, true);
    set_navigation_focus_reset_epoch(
        scope,
        navigation,
        (intercepted && focus_reset)
            .then_some(focus_reset_epoch)
            .flatten(),
    );
    set_navigation_scroll_state(scope, navigation, event, href, intercepted);
    let (redirected_url, redirected_history, redirected_state) =
        navigation_precommit_redirect(scope, event);
    let intercept_error =
        navigate_event_private_value(scope, event, NAVIGATE_EVENT_INTERCEPT_ERROR_SLOT);
    let intercept_result =
        navigate_event_private_value(scope, event, NAVIGATE_EVENT_INTERCEPT_RESULT_SLOT);
    let abort_error = navigate_event_private_value(scope, event, NAVIGATE_EVENT_ABORT_ERROR_SLOT);
    NavigationDispatchOutcome {
        proceed,
        intercepted,
        signal: signal_object,
        destination: Some(destination),
        redirected_url,
        redirected_history,
        redirected_state,
        precommit_event: (precommit_seen || intercepted).then_some(event),
        precommit_error,
        precommit_result,
        intercept_error,
        intercept_result,
        abort_error,
    }
}

pub(super) fn finish_navigation_precommit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    if let Some(controller) =
        get_private_value(scope, event, NAVIGATE_EVENT_PRECOMMIT_CONTROLLER_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_private_value(
            scope,
            controller,
            "__lmPrecommitControllerActive",
            v8::Boolean::new(scope, false).into(),
        );
        set_private_value(
            scope,
            event,
            NAVIGATE_EVENT_PRECOMMIT_CONTROLLER_SLOT,
            v8::undefined(scope).into(),
        );
    }
}

pub(super) fn navigation_precommit_redirect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> (
    Option<String>,
    Option<String>,
    Option<v8::Local<'s, v8::Value>>,
) {
    if !navigate_event_private_bool(scope, event, NAVIGATE_EVENT_REDIRECTED_SLOT, false) {
        return (None, None, None);
    }
    let destination = event
        .get(scope, v8str(scope, "destination").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let url =
        destination.and_then(|destination| navigation_destination_url_value(scope, destination));
    let history = navigate_event_private_value(scope, event, NAVIGATE_EVENT_REDIRECT_HISTORY_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| matches!(value.as_str(), "push" | "replace"));
    let state =
        destination.and_then(|destination| navigation_destination_state(scope, destination));
    (url, history, state)
}

pub(super) fn run_navigation_precommit_deferred_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> (
    Option<v8::Local<'s, v8::Value>>,
    Option<v8::Local<'s, v8::Value>>,
) {
    let arguments = [];
    run_navigation_handler_arrays(
        scope,
        event,
        &[
            NAVIGATE_EVENT_DEFERRED_HANDLERS_SLOT,
            NAVIGATE_EVENT_ADDED_HANDLERS_SLOT,
        ],
        &arguments,
    )
}

pub(crate) fn dispatch_cross_document_navigation_navigate_event_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    download_request: Option<&str>,
) -> bool {
    dispatch_cross_document_navigation_navigate_event_for_window_with_form_data(
        scope,
        owner,
        href,
        source_element,
        user_initiated,
        download_request,
        None,
    )
}

pub(crate) fn dispatch_cross_document_navigation_navigate_event_for_window_with_form_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    download_request: Option<&str>,
    form_data: Option<v8::Local<'s, v8::Value>>,
) -> bool {
    dispatch_cross_document_navigation_navigate_event_for_window_with_type_and_form_data(
        scope,
        owner,
        href,
        "push",
        source_element,
        user_initiated,
        download_request,
        form_data,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_cross_document_navigation_navigate_event_for_window_with_type_and_form_data<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    navigation_type: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    download_request: Option<&str>,
    form_data: Option<v8::Local<'s, v8::Value>>,
) -> bool {
    dispatch_cross_document_navigation_navigate_event_for_window_with_type_form_data_and_intercept(
        scope,
        owner,
        href,
        navigation_type,
        source_element,
        user_initiated,
        download_request,
        form_data,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn dispatch_cross_document_navigation_navigate_event_for_window_with_type_form_data_and_intercept<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    href: &str,
    navigation_type: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
    download_request: Option<&str>,
    form_data: Option<v8::Local<'s, v8::Value>>,
    can_intercept: bool,
) -> bool {
    super::navigation_interception::dispatch_cross_document_navigation_event(
        scope,
        owner,
        href,
        navigation_type,
        source_element,
        user_initiated,
        download_request,
        form_data,
        can_intercept,
    )
}

pub(crate) fn dispatch_srcdoc_navigation_navigate_event_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    dispatch_cross_document_navigation_navigate_event_for_window_with_type_form_data_and_intercept(
        scope,
        owner,
        "about:srcdoc",
        "push",
        None,
        false,
        None,
        None,
        false,
    )
}

pub(super) fn dispatch_navigation_traverse_event_with_outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    history: v8::Local<'s, v8::Object>,
    target_index: u32,
    info: Option<v8::Local<'s, v8::Value>>,
) -> NavigationDispatchOutcome<'s> {
    let context = navigation
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let focus_reset_epoch = context_host_ptr_from_global_bridge(scope)
        .map(|host_ptr| unsafe { &*host_ptr }.focus_change_epoch());
    let Ok(event_ctor) =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "NavigateEvent")
    else {
        return NavigationDispatchOutcome::proceed();
    };
    let Some(entries) = history_entries(scope, history) else {
        return NavigationDispatchOutcome::proceed();
    };
    let Some(entry) = entries.get(target_index as usize) else {
        return NavigationDispatchOutcome::proceed();
    };
    let owner = runtime_window_owner(scope, navigation);
    let entry = super::history_runtime::native::entry_wrapper(scope, owner, entry.clone());
    // Traversal event eligibility uses the destination Document's old origin,
    // before fetching it (and possibly redirecting). It is independent of the
    // contiguous same-origin region exposed by navigation.entries().
    if navigation_has_disabled_entries(scope, navigation)
        || !navigation_current_entry(scope, owner)
            .is_some_and(|current| navigation_entries_share_origin(scope, current, entry))
    {
        return NavigationDispatchOutcome::proceed();
    }
    let destination = create_navigation_destination_for_entry(scope, navigation, entry);
    let target_href = navigation_destination_url_value(scope, destination).unwrap_or_default();
    let owner = runtime_window_owner(scope, navigation);
    let current_href = window_location_for_holder(scope, owner)
        .and_then(|location| location_href_slot(scope, location))
        .unwrap_or_default();
    let destination_same_document = navigation_destination_same_document_value(scope, destination);
    let hash_change =
        destination_same_document && should_dispatch_hash_change(&current_href, &target_href);
    let signal = create_navigation_abort_signal(scope);
    let signal_object = v8::Local::<v8::Object>::try_from(signal).ok();
    let cancelable = destination_same_document
        && child_browsing_context_handle_for_runtime_owner(scope, owner).is_none();
    let init = NavigateEventInitDeclaration {
        navigation_type: v8str(scope, "traverse").into(),
        destination,
        can_intercept: destination_same_document,
        user_initiated: false,
        hash_change,
        signal,
        form_data: v8::null(scope).into(),
        download_request: v8::null(scope).into(),
        info: info.unwrap_or_else(|| v8::undefined(scope).into()),
        source_element: v8::null(scope).into(),
        cancelable,
    }
    .bind(scope)
    .expect("NavigateEvent init declaration should bind");
    let Some(event) =
        event_ctor.new_instance(scope, &[v8str(scope, "navigate").into(), init.into()])
    else {
        return NavigationDispatchOutcome::proceed();
    };
    mark_event_trusted(scope, event);
    set_navigate_event_private_bool(scope, event, NAVIGATE_EVENT_SYNTHETIC_SLOT, false);
    set_navigate_event_private_bool(scope, event, NAVIGATE_EVENT_FOCUS_RESET_SLOT, true);
    set_navigate_event_private_bool(
        scope,
        event,
        NAVIGATE_EVENT_SCROLL_AFTER_TRANSITION_SLOT,
        true,
    );
    install_precommit_transition_seed(scope, navigation, event, destination, "traverse");
    let href = target_href;
    let active_data = create_active_navigate_event_data(scope, event, signal_object, &href);
    set_navigation_active_navigate_event(scope, navigation, active_data);
    let owner = runtime_window_owner(scope, navigation);
    save_current_navigation_entry_scroll_position(scope, owner);
    let proceed = dispatch_simple_event_target_event(
        scope,
        navigation,
        NAVIGATION_EVENT_LISTENERS_SLOT,
        "navigate",
        event,
    );
    let (precommit_error, precommit_result) = if proceed {
        run_navigate_event_precommit_handlers(scope, event)
    } else {
        (None, None)
    };
    clear_navigation_active_navigate_event_if_current(scope, navigation, active_data);
    let precommit_seen =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_PRECOMMIT_SEEN_SLOT, false);
    let intercepted =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_INTERCEPTED_SLOT, false);
    let focus_reset =
        navigate_event_private_bool(scope, event, NAVIGATE_EVENT_FOCUS_RESET_SLOT, true);
    set_navigation_focus_reset_epoch(
        scope,
        navigation,
        (intercepted && focus_reset)
            .then_some(focus_reset_epoch)
            .flatten(),
    );
    set_navigation_scroll_state(scope, navigation, event, &href, intercepted);
    let intercept_error =
        navigate_event_private_value(scope, event, NAVIGATE_EVENT_INTERCEPT_ERROR_SLOT);
    let intercept_result =
        navigate_event_private_value(scope, event, NAVIGATE_EVENT_INTERCEPT_RESULT_SLOT);
    let abort_error = navigate_event_private_value(scope, event, NAVIGATE_EVENT_ABORT_ERROR_SLOT);
    NavigationDispatchOutcome {
        proceed,
        intercepted,
        signal: signal_object,
        destination: Some(destination),
        redirected_url: None,
        redirected_history: None,
        redirected_state: None,
        precommit_event: (precommit_seen || intercepted).then_some(event),
        precommit_error,
        precommit_result,
        intercept_error,
        intercept_result,
        abort_error,
    }
}

fn create_navigation_destination<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    href: &str,
    same_document: bool,
    state: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Object> {
    NavigationDestinationDeclaration::new(
        v8_string(scope, href).unwrap_or_else(|| v8::String::empty(scope)),
        same_document,
        None,
        state.unwrap_or_else(|| v8::null(scope).into()),
    )
    .bind(scope)
    .expect("NavigationDestination declaration should bind")
}

fn navigation_destination_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    crate::util::get_private_object(scope, destination, NAVIGATION_DESTINATION_BACKING_SLOT)
        .unwrap_or(destination)
}

pub(super) fn navigation_destination_for_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let destination = navigation_destination_backing(scope, destination);
    let context = scope.get_current_context();
    if super::world_wrappers::belongs_to_world(scope, destination, context) {
        return Some(destination);
    }
    if let Some(wrapper) = super::world_wrappers::get(scope, destination, context) {
        return Some(wrapper);
    }
    let wrapper = create_navigation_destination(scope, "", false, None);
    set_private_value(scope, wrapper, NAVIGATION_DESTINATION_BACKING_SLOT, destination.into());
    super::world_wrappers::insert(scope, destination, wrapper);
    Some(wrapper)
}

fn create_navigation_destination_for_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let url = navigation_destination_entry_string_property(scope, entry, "url").unwrap_or_default();
    let owner = runtime_window_owner(scope, navigation);
    let same_document = navigation_current_entry(scope, owner)
        .is_some_and(|current| navigation_entries_share_document(scope, current, entry));
    let visible_entry = window_history_for_holder(scope, owner)
        .and_then(|history| history_entries(scope, history))
        .and_then(|entries| {
            let current = navigation_current_entry(scope, owner);
            super::navigation_projection::visible_navigation_index_for_entry(
                scope, entries, current, entry,
            )
        })
        .map(|_| entry);
    let state = match visible_entry {
        Some(entry) => clone_navigation_entry_state(scope, entry)
            .unwrap_or_else(|| v8::undefined(scope).into()),
        None => v8::null(scope).into(),
    };
    NavigationDestinationDeclaration::new(
        v8_string(scope, &url).unwrap_or_else(|| v8::String::empty(scope)),
        same_document,
        visible_entry,
        state,
    )
    .bind(scope)
    .expect("entry-backed NavigationDestination declaration should bind")
}

fn navigation_destination_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let destination = navigation_destination_backing(scope, destination);
    get_private_value(scope, destination, NAVIGATION_DESTINATION_ENTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn navigation_destination_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let destination =
        crate::util::get_private_object(scope, destination, NAVIGATION_DESTINATION_BACKING_SLOT)
            .unwrap_or(destination);
    get_private_value(scope, destination, NAVIGATION_DESTINATION_STATE_SLOT)
        .filter(|value| !value.is_undefined())
}

fn navigation_destination_entry_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> bool {
    let owner = runtime_window_owner(scope, entry);
    navigation_document_is_active(scope, owner)
}

pub(super) fn set_navigation_destination_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination: v8::Local<'s, v8::Object>,
    url: &str,
) {
    let destination = navigation_destination_backing(scope, destination);
    let value = v8_string(scope, url).unwrap_or_else(|| v8::String::empty(scope));
    set_private_value(
        scope,
        destination,
        NAVIGATION_DESTINATION_URL_SLOT,
        value.into(),
    );
}

pub(super) fn navigation_destination_url_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let destination = navigation_destination_backing(scope, destination);
    get_private_value(scope, destination, NAVIGATION_DESTINATION_URL_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope))
}

fn navigation_destination_url_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let destination = navigation_destination_backing(scope, args.this());
    if let Some(value) = get_private_value(scope, destination, NAVIGATION_DESTINATION_URL_SLOT) {
        rv.set(value);
    }
}

fn navigation_destination_same_document_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(navigation_destination_same_document_value(
        scope,
        args.this(),
    ));
}

fn navigation_destination_same_document_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination: v8::Local<'s, v8::Object>,
) -> bool {
    let destination = navigation_destination_backing(scope, destination);
    get_private_value(
        scope,
        destination,
        NAVIGATION_DESTINATION_SAME_DOCUMENT_SLOT,
    )
    .is_some_and(|value| value.is_true())
}

fn navigation_destination_key_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = navigation_destination_entry_token(scope, args.this(), "key");
    rv.set(value);
}

fn navigation_destination_id_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = navigation_destination_entry_token(scope, args.this(), "id");
    rv.set(value);
}

fn navigation_destination_entry_token<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    destination: v8::Local<'s, v8::Object>,
    property: &'static str,
) -> v8::Local<'s, v8::Value> {
    let Some(entry) = navigation_destination_entry(scope, destination) else {
        return v8str(scope, "").into();
    };
    if !navigation_destination_entry_is_active(scope, entry) {
        return v8str(scope, "").into();
    }
    navigation_destination_entry_string_property(scope, entry, property)
        .and_then(|value| v8_string(scope, &value))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8str(scope, "").into())
}

fn navigation_destination_index_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let index = navigation_destination_entry(scope, args.this())
        .map(|entry| navigation_entry_index_value(scope, entry))
        .unwrap_or(-1);
    rv.set(v8::Number::new(scope, index as f64).into());
}

fn navigation_destination_entry_string_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> Option<String> {
    match key {
        "id" => navigation_entry_id_value(scope, entry),
        "key" => navigation_entry_key_value(scope, entry),
        "url" => navigation_entry_url_value(scope, entry),
        _ => None,
    }
}

fn create_navigation_abort_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    let Ok(controller_ctor) =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "AbortController")
    else {
        return v8::null(scope).into();
    };
    let Some(controller) = controller_ctor.new_instance(scope, &[]) else {
        return v8::null(scope).into();
    };
    controller
        .get(scope, v8str(scope, "signal").into())
        .unwrap_or_else(|| v8::null(scope).into())
}

fn navigation_destination_get_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let destination = navigation_destination_backing(scope, args.this());
    let state = get_private_value(scope, destination, NAVIGATION_DESTINATION_STATE_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    if let Some(value) = structured_clone_value(scope, state) {
        rv.set(value);
    }
}
