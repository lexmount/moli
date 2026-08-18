use super::location_history_storage::{
    NAVIGATION_ACTIVATION_SLOT, NAVIGATION_CURRENT_ENTRY_SLOT, NAVIGATION_TRANSITION_SLOT,
};
use super::navigation_entry::{
    create_navigation_entry, history_entries, navigation_entry_public_token,
    set_navigation_entry_document_id,
};
use super::navigation_lifecycle::enqueue_navigation_lifecycle_microtask;
use super::navigation_result::suppress_unhandled_rejection;
use super::navigation_window::{
    runtime_window_owner, set_runtime_window_owner, window_history_for_holder,
};
use super::*;
use crate::native_bridge::{NavigationActivationSeed, NavigationHistorySerializedEntry};
use crate::util::{get_private_value, set_private_value};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = "NavigationActivation",
    own_to_string_tag = "NavigationActivation",
    readonly_to_string_tag
)]
struct NavigationActivationObjectDeclaration<'scope> {
    #[webapi(data_property)]
    entry: v8::Local<'scope, v8::Object>,

    #[webapi(data_property)]
    from: v8::Local<'scope, v8::Value>,

    #[webapi(data_property = "navigationType")]
    navigation_type: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "NavigationTransition",
    own_to_string_tag = "NavigationTransition",
    readonly_to_string_tag
)]
struct NavigationTransitionObjectDeclaration<'scope> {
    #[webapi(data_property)]
    from: v8::Local<'scope, v8::Object>,

    #[webapi(data_property)]
    to: v8::Local<'scope, v8::Value>,

    #[webapi(data_property = "navigationType")]
    navigation_type: &'static str,

    #[webapi(data_property)]
    committed: v8::Local<'scope, v8::Promise>,

    #[webapi(slot = NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT)]
    committed_resolver: v8::Local<'scope, v8::PromiseResolver>,

    #[webapi(data_property)]
    finished: v8::Local<'scope, v8::Promise>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct NavigationTransitionSettleDataDeclaration<'scope> {
    #[webapi(slot = NAVIGATION_TRANSITION_SETTLE_NAVIGATION_SLOT)]
    navigation: v8::Local<'scope, v8::Object>,

    #[webapi(slot = NAVIGATION_TRANSITION_SETTLE_RESOLVER_SLOT)]
    resolver: v8::Local<'scope, v8::PromiseResolver>,

    #[webapi(slot = NAVIGATION_TRANSITION_SETTLE_ERROR_SLOT)]
    error: Option<v8::Local<'scope, v8::Value>>,
}

pub(super) fn install_navigation_activation_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name != "NavigationTransition" {
        return;
    }
    template.prototype_template(scope).set_with_attr(
        v8str(scope, "committed").into(),
        v8::undefined(scope).into(),
        v8::PropertyAttribute::DONT_ENUM,
    );
}

pub(super) fn current_entry_matches_activation_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    current_entry: v8::Local<'s, v8::Object>,
    snapshot: &NavigationHistorySerializedEntry,
) -> bool {
    navigation_entry_matches_activation_snapshot(scope, current_entry, snapshot)
}

fn navigation_entry_matches_activation_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    snapshot: &NavigationHistorySerializedEntry,
) -> bool {
    let current_id = get_own_static_property(scope, entry, "id")
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    let current_key = get_own_static_property(scope, entry, "key")
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    let snapshot_id = navigation_entry_public_token(&snapshot.id);
    let snapshot_key = navigation_entry_public_token(&snapshot.key);
    current_id.as_deref() == Some(snapshot_id.as_str())
        && current_key.as_deref() == Some(snapshot_key.as_str())
}

pub(super) fn navigation_entry_object_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    current_entry: v8::Local<'s, v8::Object>,
    snapshot: &NavigationHistorySerializedEntry,
) -> v8::Local<'s, v8::Object> {
    if current_entry_matches_activation_snapshot(scope, current_entry, snapshot) {
        return current_entry;
    }
    let owner = runtime_window_owner(scope, current_entry);
    if let Some(history) = window_history_for_holder(scope, owner)
        && let Some(entries) = history_entries(scope, history)
    {
        for index in 0..entries.length() {
            let Some(entry) = entries
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            else {
                continue;
            };
            if navigation_entry_matches_activation_snapshot(scope, entry, snapshot) {
                return entry;
            }
        }
    }
    let entry = create_navigation_entry(
        scope,
        &snapshot.url,
        snapshot.history_state_json.as_deref(),
        snapshot.navigation_state_json.as_deref(),
        snapshot.referrer_policy.as_deref(),
        snapshot.index,
        &snapshot.id,
        &snapshot.key,
    );
    set_navigation_entry_document_id(scope, entry, snapshot.document_id.as_str());
    bind_navigation_entry_runtime_owner(scope, entry, owner);
    entry
}

fn navigation_activation_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    current_entry: v8::Local<'s, v8::Object>,
    activation: &NavigationActivationSeed,
) -> v8::Local<'s, v8::Object> {
    if activation.from.is_none() && activation.navigation_type.as_deref() == Some("replace") {
        return current_entry;
    }
    navigation_entry_object_from_snapshot(scope, current_entry, &activation.entry)
}

pub(super) fn bind_navigation_entry_runtime_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    set_runtime_window_owner(scope, entry, owner);
}

pub(super) fn create_navigation_activation_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    current_entry: v8::Local<'s, v8::Object>,
    activation: Option<&NavigationActivationSeed>,
) -> v8::Local<'s, v8::Value> {
    let Some(activation) = activation else {
        return v8::null(scope).into();
    };
    let entry = navigation_activation_entry_object(scope, current_entry, activation);
    let from = activation
        .from
        .as_ref()
        .map(|snapshot| navigation_entry_object_from_snapshot(scope, current_entry, snapshot))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    let navigation_type = activation
        .navigation_type
        .as_deref()
        .and_then(|value| v8_string(scope, value))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    NavigationActivationObjectDeclaration::new(entry, from, navigation_type)
        .bind(scope)
        .expect("NavigationActivation declaration should bind")
        .into()
}

pub(super) fn install_navigation_activation_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    current_entry: v8::Local<'s, v8::Object>,
    activation: Option<&NavigationActivationSeed>,
) {
    let activation_value = create_navigation_activation_object(scope, current_entry, activation);
    let transition_value = v8::null(scope).into();
    set_navigation_activation_value(scope, navigation, activation_value);
    set_navigation_transition_value(scope, navigation, transition_value);
}

pub(super) fn install_navigation_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    from: v8::Local<'s, v8::Object>,
    to: Option<v8::Local<'s, v8::Object>>,
    navigation_type: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let finished = resolver.get_promise(scope);
    suppress_unhandled_rejection(scope, finished);
    let committed_resolver = v8::PromiseResolver::new(scope)?;
    let committed = committed_resolver.get_promise(scope);
    suppress_unhandled_rejection(scope, committed);
    let transition = NavigationTransitionObjectDeclaration {
        from,
        to: to
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        navigation_type,
        committed,
        committed_resolver,
        finished,
    }
    .bind(scope)
    .expect("NavigationTransition declaration should bind");
    set_navigation_transition_value(scope, navigation, transition.into());
    Some(resolver)
}

pub(super) fn resolve_navigation_transition_committed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    if let Some(resolver) = take_navigation_transition_committed_resolver(scope, navigation) {
        let _ = resolver.resolve(scope, value);
    }
}

pub(super) fn reject_navigation_transition_committed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    if let Some(resolver) = take_navigation_transition_committed_resolver(scope, navigation) {
        let _ = resolver.reject(scope, error);
    }
}

pub(super) fn clear_navigation_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) {
    set_navigation_transition_value(scope, navigation, v8::null(scope).into());
}

const NAVIGATION_TRANSITION_SETTLE_NAVIGATION_SLOT: &str =
    "__lmNavigationTransitionSettleNavigation";
const NAVIGATION_TRANSITION_SETTLE_RESOLVER_SLOT: &str = "__lmNavigationTransitionSettleResolver";
const NAVIGATION_TRANSITION_SETTLE_ERROR_SLOT: &str = "__lmNavigationTransitionSettleError";
const NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT: &str =
    "__lmNavigationTransitionCommittedResolver";

fn take_navigation_transition_committed_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let transition = navigation_transition_object(scope, navigation)?;
    let resolver = get_private_value(
        scope,
        transition,
        NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })?;
    set_private_value(
        scope,
        transition,
        NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT,
        v8::undefined(scope).into(),
    );
    Some(resolver)
}

pub(super) fn schedule_settle_navigation_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    error: Option<v8::Local<'s, v8::Value>>,
) {
    let data = NavigationTransitionSettleDataDeclaration {
        navigation,
        resolver,
        error,
    }
    .bind(scope)
    .expect("navigation transition settle data should bind");
    let Some(callback) = v8::Function::builder(settle_navigation_transition_callback)
        .data(data.into())
        .build(scope)
    else {
        clear_navigation_transition(scope, navigation);
        return;
    };
    enqueue_navigation_lifecycle_microtask(scope, callback);
}

fn settle_navigation_transition_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let Some(navigation) =
        get_private_value(scope, data, NAVIGATION_TRANSITION_SETTLE_NAVIGATION_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(resolver) = get_private_value(scope, data, NAVIGATION_TRANSITION_SETTLE_RESOLVER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
    else {
        return;
    };
    if navigation_transition_matches_resolver(scope, navigation, resolver) {
        clear_navigation_transition(scope, navigation);
    }
    if let Some(error) = get_private_value(scope, data, NAVIGATION_TRANSITION_SETTLE_ERROR_SLOT)
        .filter(|value| !value.is_undefined())
    {
        let _ = resolver.reject(scope, error);
    } else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
    }
}

fn navigation_transition_matches_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> bool {
    let current_finished = navigation_transition_object(scope, navigation)
        .and_then(|transition| transition.get(scope, v8str(scope, "finished").into()));
    current_finished
        .is_some_and(|finished| finished.strict_equals(resolver.get_promise(scope).into()))
}

pub(super) fn navigation_current_entry_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, navigation, NAVIGATION_CURRENT_ENTRY_SLOT)
        .filter(|value| !value.is_undefined())
}

pub(super) fn navigation_activation_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, navigation, NAVIGATION_ACTIVATION_SLOT)
        .filter(|value| !value.is_undefined())
}

pub(super) fn navigation_transition_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, navigation, NAVIGATION_TRANSITION_SLOT)
        .filter(|value| !value.is_undefined())
}

fn navigation_transition_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    navigation_transition_value(scope, navigation)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn set_navigation_activation_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, navigation, NAVIGATION_ACTIVATION_SLOT, value);
}

fn set_navigation_transition_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, navigation, NAVIGATION_TRANSITION_SLOT, value);
}

pub(super) fn set_navigation_current_entry(
    scope: &mut v8::PinScope<'_, '_>,
    navigation: v8::Local<'_, v8::Object>,
    entry: v8::Local<'_, v8::Object>,
) {
    set_private_value(
        scope,
        navigation,
        NAVIGATION_CURRENT_ENTRY_SLOT,
        entry.into(),
    );
}
