//! Value bindings for navigation. The embedding runtime owns history and promise settlement.

pub mod options;

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_object, get_private_value, set_private_value, v8_string, v8str};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

const NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT: &str =
    "__lmNavigationTransitionCommittedResolver";

pub const NAVIGATION_DESTINATION_URL_SLOT: &str = "__lmNavigationDestinationUrl";
pub const NAVIGATION_DESTINATION_SAME_DOCUMENT_SLOT: &str = "__lmNavigationDestinationSameDocument";
pub const NAVIGATION_DESTINATION_ENTRY_SLOT: &str = "__lmNavigationDestinationEntry";
pub const NAVIGATION_DESTINATION_STATE_SLOT: &str = "__lmNavigationDestinationState";

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

const ACTIVATION_ENTRY: &str = "__lmNavigationActivationEntry";
const ACTIVATION_FROM: &str = "__lmNavigationActivationFrom";
const ACTIVATION_TYPE: &str = "__lmNavigationActivationType";
const TRANSITION_FROM: &str = "__lmNavigationTransitionFrom";
const TRANSITION_TO: &str = "__lmNavigationTransitionTo";
const TRANSITION_TYPE: &str = "__lmNavigationTransitionType";
const TRANSITION_COMMITTED: &str = "__lmNavigationTransitionCommitted";
const TRANSITION_FINISHED: &str = "__lmNavigationTransitionFinished";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigationActivation, require_prototype)]
pub struct NavigationActivationObjectDeclaration<'s> {
    #[webapi(slot = ACTIVATION_ENTRY)]
    pub entry: v8::Local<'s, v8::Object>,
    #[webapi(slot = ACTIVATION_FROM)]
    pub from: v8::Local<'s, v8::Value>,
    #[webapi(slot = ACTIVATION_TYPE)]
    pub navigation_type: v8::Local<'s, v8::Value>,
}

impl<'s> NavigationActivationObjectDeclaration<'s> {
    pub fn from_private_state(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        Some(Self {
            entry: get_private_object(scope, object, ACTIVATION_ENTRY)?,
            from: get_private_value(scope, object, ACTIVATION_FROM)?,
            navigation_type: get_private_value(scope, object, ACTIVATION_TYPE)?,
        })
    }
}

// These are associated platform-object fields, not an author dictionary.
// World views keep their own projected handles and share settlement authority
// only through the canonical transition retained by Navigation.
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigationTransition, require_prototype)]
pub struct NavigationTransitionObjectDeclaration<'s> {
    #[webapi(slot = TRANSITION_FROM)]
    pub from: v8::Local<'s, v8::Object>,
    #[webapi(slot = TRANSITION_TO)]
    pub to: v8::Local<'s, v8::Value>,
    #[webapi(slot = TRANSITION_TYPE)]
    navigation_type: v8::Local<'s, v8::Value>,
    #[webapi(slot = TRANSITION_COMMITTED)]
    pub committed: v8::Local<'s, v8::Promise>,
    #[webapi(slot = NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT)]
    committed_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
    #[webapi(slot = TRANSITION_FINISHED)]
    pub finished: v8::Local<'s, v8::Promise>,
}

impl<'s> NavigationTransitionObjectDeclaration<'s> {
    pub fn from_private_state(
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
            finished: transition_finished(scope, object)?,
        })
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NavigationActivation, enumerable)]
struct NavigationActivationPrototypeDeclaration {
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, ACTIVATION_FROM))]
    from: (),
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, ACTIVATION_ENTRY))]
    entry: (),
    #[webapi(accessor_property, getter = associated_field_getter, data = v8str(scope, ACTIVATION_TYPE))]
    navigation_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NavigationTransition, enumerable)]
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
struct NavigationResultDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    committed: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property, enumerable)]
    finished: v8::Local<'scope, v8::Promise>,
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct NavigationResultFallbackDeclaration {
    #[webapi(data_property, enumerable, name = "committed", init = "undefined")]
    committed: (),
    #[webapi(data_property, enumerable, name = "finished", init = "undefined")]
    finished: (),
}

pub fn install_navigation_activation_template_bindings<'s>(
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

pub fn new_navigation_activation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    from: v8::Local<'s, v8::Value>,
    navigation_type: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    NavigationActivationObjectDeclaration::new(entry, from, navigation_type)
        .bind(scope)
        .expect("NavigationActivation declaration should bind")
}

pub fn new_navigation_transition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    from: v8::Local<'s, v8::Object>,
    to: Option<v8::Local<'s, v8::Object>>,
    navigation_type: &'static str,
    committed_resolver: v8::Local<'s, v8::PromiseResolver>,
    finished: v8::Local<'s, v8::Promise>,
) -> v8::Local<'s, v8::Object> {
    NavigationTransitionObjectDeclaration {
        from,
        to: to
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        navigation_type: v8str(scope, navigation_type).into(),
        committed: committed_resolver.get_promise(scope),
        committed_resolver: Some(committed_resolver),
        finished,
    }
    .bind(scope)
    .expect("NavigationTransition declaration should bind")
}

pub fn transition_finished<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    get_private_value(scope, transition, TRANSITION_FINISHED)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
}

pub fn new_navigation_destination<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    href: &str,
    same_document: bool,
    entry: Option<v8::Local<'s, v8::Object>>,
    state: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    NavigationDestinationDeclaration::new(
        v8_string(scope, href).unwrap_or_else(|| v8::String::empty(scope)),
        same_document,
        entry,
        state,
    )
    .bind(scope)
    .expect("NavigationDestination declaration should bind")
}

pub fn take_transition_committed_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
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

pub fn navigation_result_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    committed: v8::Local<'s, v8::Promise>,
    finished: v8::Local<'s, v8::Promise>,
) -> v8::Local<'s, v8::Object> {
    NavigationResultDeclaration::new(committed, finished)
        .bind(scope)
        .expect("NavigationResult declaration should bind")
}

pub fn navigation_result_fallback_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    NavigationResultFallbackDeclaration::default()
        .bind(scope)
        .expect("NavigationResult fallback declaration should bind")
}
