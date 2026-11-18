//! Value bindings for navigation. The embedding runtime owns history and promise settlement.

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, set_private_value, v8str};
use moli_webapi_declare::WebApiObject;

const NAVIGATION_TRANSITION_COMMITTED_RESOLVER_SLOT: &str =
    "__lmNavigationTransitionCommittedResolver";

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::NavigationActivation,
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
    interface = web_api_interfaces::NavigationTransition,
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
        navigation_type,
        committed: committed_resolver.get_promise(scope),
        committed_resolver,
        finished,
    }
    .bind(scope)
    .expect("NavigationTransition declaration should bind")
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
