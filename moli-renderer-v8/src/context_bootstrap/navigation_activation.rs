use super::history_runtime::native;
use super::location_history_storage::{
    NAVIGATION_ACTIVATION_SLOT, NAVIGATION_CURRENT_ENTRY_SLOT, NAVIGATION_TRANSITION_SLOT,
};
use super::navigation_entry::{history_entries, navigation_entry_public_token};
use super::navigation_lifecycle::enqueue_navigation_lifecycle_microtask;
use super::navigation_result::suppress_unhandled_rejection;
use super::navigation_seed::history_entry_from_snapshot;
use super::navigation_window::{runtime_window_owner, window_history_for_holder};
use super::*;
use crate::context_bootstrap::navigation_entry::wrappers as entry_wrappers;
use crate::native_bridge::{NavigationActivationSeed, NavigationHistorySerializedEntry};
use crate::util::{get_private_object, get_private_value, set_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

pub(super) const NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT: &str =
    "__lmNavigateEventPrecommitTransitionResolver";

const ACTIVATION_ENTRY: &str = "__lmNavigationActivationEntry";
const ACTIVATION_FROM: &str = "__lmNavigationActivationFrom";
const ACTIVATION_TYPE: &str = "__lmNavigationActivationType";
const ACTIVATION_SOURCE_INITIAL: &str = "__lmNavigationActivationSourceInitial";
const TRANSITION_FROM: &str = "__lmNavigationTransitionFrom";
const TRANSITION_TO: &str = "__lmNavigationTransitionTo";
const TRANSITION_TYPE: &str = "__lmNavigationTransitionType";
const TRANSITION_COMMITTED: &str = "__lmNavigationTransitionCommitted";
const TRANSITION_FINISHED: &str = "__lmNavigationTransitionFinished";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigationActivation, require_prototype)]
pub(super) struct NavigationActivationObjectDeclaration<'s> {
    #[webapi(slot = ACTIVATION_ENTRY)]
    pub(super) entry: v8::Local<'s, v8::Object>,
    #[webapi(slot = ACTIVATION_FROM)]
    pub(super) from: v8::Local<'s, v8::Value>,
    #[webapi(slot = ACTIVATION_TYPE)]
    pub(super) navigation_type: v8::Local<'s, v8::Value>,
    #[webapi(slot = ACTIVATION_SOURCE_INITIAL)]
    pub(super) source_was_initial: bool,
}

impl<'s> NavigationActivationObjectDeclaration<'s> {
    pub(super) fn from_private_state(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        Some(Self {
            entry: get_private_object(scope, object, ACTIVATION_ENTRY)?,
            from: get_private_value(scope, object, ACTIVATION_FROM)?,
            navigation_type: get_private_value(scope, object, ACTIVATION_TYPE)?,
            source_was_initial: get_private_value(scope, object, ACTIVATION_SOURCE_INITIAL)?
                .boolean_value(scope),
        })
    }
}

// These are associated platform-object fields, not an author dictionary.
// World views keep their own projected handles and share settlement authority
// only through the canonical transition retained by Navigation.
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigationTransition, require_prototype)]
pub(super) struct NavigationTransitionObjectDeclaration<'s> {
    #[webapi(slot = TRANSITION_FROM)]
    pub(super) from: v8::Local<'s, v8::Object>,
    #[webapi(slot = TRANSITION_TO)]
    pub(super) to: v8::Local<'s, v8::Value>,
    #[webapi(slot = TRANSITION_TYPE)]
    navigation_type: v8::Local<'s, v8::Value>,
    #[webapi(slot = TRANSITION_COMMITTED)]
    pub(super) committed: v8::Local<'s, v8::Promise>,
    #[webapi(slot = NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT)]
    committed_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    #[webapi(slot = TRANSITION_FINISHED)]
    pub(super) finished: v8::Local<'s, v8::Promise>,
}

impl<'s> NavigationTransitionObjectDeclaration<'s> {
    pub(super) fn from_private_state(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        Some(Self {
            from: get_private_object(scope, object, TRANSITION_FROM)?,
            to: get_private_value(scope, object, TRANSITION_TO)?,
            navigation_type: get_private_value(scope, object, TRANSITION_TYPE)?,
            committed: get_private_value(scope, object, TRANSITION_COMMITTED)
                .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())?,
            committed_resolver: None,
            finished: get_private_value(scope, object, TRANSITION_FINISHED)
                .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())?,
        })
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NavigationActivation, enumerable, receiver)]
struct NavigationActivationPrototypeDeclaration {
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, ACTIVATION_FROM))]
    from: (),
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, ACTIVATION_ENTRY))]
    entry: (),
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, ACTIVATION_TYPE))]
    navigation_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NavigationTransition, enumerable, receiver)]
struct NavigationTransitionPrototypeDeclaration {
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, TRANSITION_TYPE))]
    navigation_type: (),
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, TRANSITION_FROM))]
    from: (),
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, TRANSITION_TO))]
    to: (),
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, TRANSITION_COMMITTED), returns_promise)]
    committed: (),
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, TRANSITION_FINISHED), returns_promise)]
    finished: (),
}

fn associated_field_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = web_api_object_target(scope, args.this()).expect("validated navigation receiver");
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, object, &slot).expect("navigation associated field"));
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NavigationTransitionSettleDataDeclaration<'scope> {
    #[webapi(slot = NAVIGATION_TRANSITION_SETTLE_NAVIGATION_SLOT)]
    navigation: v8::Local<'scope, v8::Object>,

    #[webapi(slot = NAVIGATION_TRANSITION_SETTLE_RESOLVER_SLOT)]
    resolver: v8::Local<'scope, v8::PromiseResolver>,

    #[webapi(slot = NAVIGATION_TRANSITION_SETTLE_ERROR_SLOT)]
    error: Option<v8::Local<'scope, v8::Value>>,

    #[webapi(slot = NAVIGATION_TRANSITION_SETTLE_REJECTED_SLOT)]
    rejected: bool,
}

pub(super) fn install_navigation_activation_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "NavigationActivation" => {
            NavigationActivationPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "NavigationTransition" => {
            NavigationTransitionPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        _ => {}
    }
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
    native::entry(scope, entry)
        .is_some_and(|entry| native_entry_matches_activation_snapshot(&entry.borrow(), snapshot))
}

fn native_entry_matches_activation_snapshot(
    entry: &moli_history::HistoryEntry,
    snapshot: &NavigationHistorySerializedEntry,
) -> bool {
    entry.id == navigation_entry_public_token(&snapshot.id)
        && entry.key.as_str() == navigation_entry_public_token(&snapshot.key)
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
        for entry in entries {
            if native_entry_matches_activation_snapshot(&entry.borrow(), snapshot) {
                return entry_wrappers::for_window(scope, owner, entry);
            }
        }
    }
    entry_wrappers::for_window(scope, owner, history_entry_from_snapshot(snapshot))
}

pub(super) fn create_navigation_activation_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
    current_entry: v8::Local<'s, v8::Object>,
    activation: Option<&NavigationActivationSeed>,
) -> v8::Local<'s, v8::Value> {
    let Some(activation) = activation else {
        return v8::null(scope).into();
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let entry = navigation_entry_object_from_snapshot(scope, current_entry, &activation.entry);
    let destination_origin =
        native::entry(scope, current_entry).map(|entry| entry.borrow().document_origin.clone());
    let from = activation
        .from
        .as_ref()
        .filter(|snapshot| {
            activation.source_document == moli_page_types::NavigationSourceDocument::NonInitial
                && snapshot.document_origin != "null"
                && destination_origin.as_deref() == Some(snapshot.document_origin.as_str())
        })
        .map(|snapshot| navigation_entry_object_from_snapshot(scope, current_entry, snapshot))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    let navigation_type = activation
        .navigation_type
        .as_deref()
        .and_then(|value| v8_string(scope, value))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    let entry = entry_wrappers::in_realm(scope, entry, context);
    let from = if let Ok(from) = v8::Local::<v8::Object>::try_from(from) {
        entry_wrappers::in_realm(scope, from, context).into()
    } else {
        from
    };
    NavigationActivationObjectDeclaration::new(
        entry,
        from,
        navigation_type,
        activation.source_document == moli_page_types::NavigationSourceDocument::InitialEmpty,
    )
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
    let context = navigation
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let activation_value =
        create_navigation_activation_object(scope, context, current_entry, activation);
    set_navigation_activation_value(scope, navigation, activation_value);
    if super::shared_event_targets::shared_target_owner(scope, navigation)
        .strict_equals(navigation.into())
    {
        set_navigation_transition_value(scope, navigation, v8::null(scope).into());
    }
}

pub(super) fn install_navigation_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    from: v8::Local<'s, v8::Object>,
    to: Option<v8::Local<'s, v8::Object>>,
    navigation_type: &'static str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    // Joint traversals can install another Window's transition from the
    // initiating realm. The transition and both promises belong to Navigation.
    let context = navigation.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let resolver = v8::PromiseResolver::new(scope)?;
    let finished = resolver.get_promise(scope);
    suppress_unhandled_rejection(scope, finished);
    let committed_resolver = v8::PromiseResolver::new(scope)?;
    let committed = committed_resolver.get_promise(scope);
    suppress_unhandled_rejection(scope, committed);
    let context = scope.get_current_context();
    let from = entry_wrappers::in_realm(scope, from, context);
    let to =
        to.and_then(|to| super::navigation_events::navigation_destination_for_realm(scope, to));
    let transition = NavigationTransitionObjectDeclaration {
        from,
        to: to
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        navigation_type: v8str(scope, navigation_type).into(),
        committed,
        committed_resolver: Some(committed_resolver),
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
) {
    if let Some(resolver) = take_navigation_transition_committed_resolver(scope, navigation) {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
    }
}

pub(super) fn precommit_transition_resolver_from_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    event_private_value(
        scope,
        event,
        NAVIGATE_EVENT_PRECOMMIT_TRANSITION_RESOLVER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    .map(|object| unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
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
const NAVIGATION_TRANSITION_SETTLE_REJECTED_SLOT: &str = "__lmNavigationTransitionSettleRejected";
const NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT: &str =
    "__lmNavigationTransitionCommittedResolver";

pub(super) fn take_navigation_transition_committed_resolver<'s>(
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
        rejected: error.is_some(),
    }
    .bind(scope)
    .expect("navigation transition settle data should bind");
    let Some(callback) = v8::Function::builder(settle_navigation_transition_callback)
        .data(data.into())
        .build(scope)
    else {
        if navigation_transition_matches_resolver(scope, navigation, resolver) {
            clear_navigation_transition(scope, navigation);
        }
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
    if get_private_value(scope, data, NAVIGATION_TRANSITION_SETTLE_REJECTED_SLOT)
        .is_some_and(|value| value.is_true())
    {
        let error = get_private_value(scope, data, NAVIGATION_TRANSITION_SETTLE_ERROR_SLOT)
            .unwrap_or_else(|| v8::undefined(scope).into());
        let _ = resolver.reject(scope, error);
    } else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
    }
}

pub(super) fn navigation_transition_matches_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> bool {
    let current_finished = navigation_transition_object(scope, navigation)
        .and_then(|transition| get_private_value(scope, transition, TRANSITION_FINISHED));
    current_finished
        .is_some_and(|finished| finished.strict_equals(resolver.get_promise(scope).into()))
}

pub(super) fn navigation_current_entry_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let owner = runtime_window_owner(scope, navigation);
    let entry = super::navigation_entry::navigation_current_entry(scope, owner)?;
    let context = navigation.get_creation_context(scope)?;
    Some(entry_wrappers::in_realm(scope, entry, context).into())
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
    let context = navigation.get_creation_context(scope)?;
    let Some(transition) = navigation_transition_object(scope, navigation) else {
        return Some(v8::null(scope).into());
    };
    super::navigation_transition_worlds::transition_in_realm(scope, transition, context)
        .map(Into::into)
}

fn navigation_transition_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let navigation = super::shared_event_targets::shared_target_owner(scope, navigation);
    get_private_value(scope, navigation, NAVIGATION_TRANSITION_SLOT)
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
    let navigation = super::shared_event_targets::shared_target_owner(scope, navigation);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_transition_and_promises_use_navigation_creation_realm() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        for (owner, caller) in [(first, second), (second, first)] {
            let (navigation, from) = {
                let scope = &mut v8::ContextScope::new(scope, owner);
                let template = NavigationTransitionPrototypeDeclaration::build(scope);
                let constructor = template.get_function(scope).unwrap();
                let global = owner.global(scope);
                assert_eq!(
                    global.create_data_property(
                        scope,
                        v8str(scope, "NavigationTransition").into(),
                        constructor.into()
                    ),
                    Some(true)
                );
                (v8::Object::new(scope), v8::Object::new(scope))
            };
            let scope = &mut v8::ContextScope::new(scope, caller);
            let resolver = install_navigation_transition(scope, navigation, from, None, "traverse")
                .expect("transition should be installed");
            assert_eq!(scope.get_current_context(), caller);
            let transition = navigation_transition_object(scope, navigation).unwrap();
            assert_eq!(transition.get_creation_context(scope), Some(owner));
            for name in ["committed", "finished"] {
                let value = transition.get(scope, v8str(scope, name).into()).unwrap();
                let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
                assert_eq!(promise.get_creation_context(scope), Some(owner), "{name}");
                if name == "finished" {
                    assert_eq!(promise, resolver.get_promise(scope));
                }
            }
            assert!(
                transition
                    .get(scope, v8str(scope, "from").into())
                    .unwrap()
                    .strict_equals(from.into())
            );
            resolve_navigation_transition_committed(scope, navigation);
            let committed = transition
                .get(scope, v8str(scope, "committed").into())
                .unwrap();
            let committed = v8::Local::<v8::Promise>::try_from(committed).unwrap();
            assert_eq!(committed.state(), v8::PromiseState::Fulfilled);
            assert!(committed.result(scope).is_undefined());
        }
    }
}
