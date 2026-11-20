//! Value bindings for navigation. The embedding runtime owns history and promise settlement.

pub mod options;

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_object, get_private_value, set_private_value, v8_string, v8str};
use moli_webapi_declare::WebApiObject;

const NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT: &str =
    "__lmNavigationTransitionCommittedResolver";

const ACTIVATION_ENTRY: &str = "__lmNavigationActivationEntry";
const ACTIVATION_FROM: &str = "__lmNavigationActivationFrom";
const ACTIVATION_TYPE: &str = "__lmNavigationActivationType";

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

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::NavigationActivation,
    own_to_string_tag = "NavigationActivation",
    readonly_to_string_tag
)]
pub struct NavigationActivationObjectDeclaration<'scope> {
    #[webapi(data_property)]
    pub entry: v8::Local<'scope, v8::Object>,

    #[webapi(data_property)]
    pub from: v8::Local<'scope, v8::Value>,

    #[webapi(data_property = "navigationType")]
    pub navigation_type: v8::Local<'scope, v8::Value>,
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

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::NavigationTransition,
    own_to_string_tag = "NavigationTransition",
    readonly_to_string_tag
)]
struct NavigationTransitionObjectDeclaration<'scope> {
    #[webapi(data_property)]
    from: v8::Local<'scope, v8::Value>,

    #[webapi(data_property)]
    to: v8::Local<'scope, v8::Value>,

    #[webapi(data_property = "navigationType")]
    navigation_type: v8::Local<'scope, v8::Value>,

    #[webapi(data_property)]
    committed: v8::Local<'scope, v8::Promise>,

    #[webapi(slot = NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT)]
    committed_resolver: Option<v8::Local<'scope, v8::PromiseResolver>>,

    #[webapi(data_property)]
    finished: v8::Local<'scope, v8::Promise>,
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
    if interface_name != "NavigationTransition" {
        return;
    }
    template.prototype_template(scope).set_with_attr(
        v8str(scope, "committed").into(),
        v8::undefined(scope).into(),
        v8::PropertyAttribute::DONT_ENUM,
    );
}

pub fn new_navigation_activation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    from: v8::Local<'s, v8::Value>,
    navigation_type: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let activation = NavigationActivationObjectDeclaration::new(entry, from, navigation_type)
        .bind(scope)
        .expect("NavigationActivation declaration should bind");
    set_private_value(scope, activation, ACTIVATION_ENTRY, entry.into());
    set_private_value(scope, activation, ACTIVATION_FROM, from);
    set_private_value(scope, activation, ACTIVATION_TYPE, navigation_type);
    activation
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
        from: from.into(),
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

pub fn new_navigation_transition_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    from: v8::Local<'s, v8::Value>,
    to: v8::Local<'s, v8::Value>,
    navigation_type: v8::Local<'s, v8::Value>,
    committed: v8::Local<'s, v8::Promise>,
    finished: v8::Local<'s, v8::Promise>,
) -> Option<v8::Local<'s, v8::Object>> {
    NavigationTransitionObjectDeclaration {
        from,
        to,
        navigation_type,
        committed,
        committed_resolver: None,
        finished,
    }
    .bind(scope)
    .ok()
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
