use super::navigator::{
    build_window_navigator_object_for_owner, navigator_identity_profile,
    set_navigator_identity_profile, update_window_navigator_identity,
};
use crate::{
    context_bootstrap::{
        GlobalCachesAccessorDeclaration, WINDOW_NAVIGATOR_SLOT,
        window_accessors::window_child_context_handle,
    },
    util::{get_private_value, set_private_value, v8_string},
};
use anyhow::{Result, anyhow};
use moli_browser_profile::BrowserIdentityProfile;

const WINDOW_NAVIGATOR_STORAGE_APIS_AVAILABLE_SLOT: &str =
    "__moliWindowNavigatorStorageApisAvailable";
const WINDOW_NAVIGATOR_USER_AGENT_SLOT: &str = "__moliWindowNavigatorUserAgent";
const WINDOW_NAVIGATOR_ACCEPT_LANGUAGE_SLOT: &str = "__moliWindowNavigatorAcceptLanguage";

pub(in crate::context_bootstrap) fn build_window_navigator_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let owner_child = window_child_context_handle(scope, receiver);
    let user_agent = get_private_value(scope, receiver, WINDOW_NAVIGATOR_USER_AGENT_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope));
    let accept_language = get_private_value(scope, receiver, WINDOW_NAVIGATOR_ACCEPT_LANGUAGE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE.to_owned());
    let identity = navigator_identity_profile(scope, receiver).or_else(|| {
        user_agent.map(|user_agent| BrowserIdentityProfile::new(user_agent, accept_language))
    });
    let storage_apis_available = get_private_value(
        scope,
        receiver,
        WINDOW_NAVIGATOR_STORAGE_APIS_AVAILABLE_SLOT,
    )
    .is_none_or(|value| value.boolean_value(scope));
    build_window_navigator_object_for_owner(
        scope,
        owner_child,
        identity.as_ref(),
        storage_apis_available,
    )
}

pub(in crate::context_bootstrap) fn install_navigator_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    storage_apis_available: bool,
) -> Result<()> {
    super::user_activation::initialize_window_user_activation(scope, global)?;
    let storage_apis_available_value = v8::Boolean::new(scope, storage_apis_available);
    set_private_value(
        scope,
        global,
        WINDOW_NAVIGATOR_STORAGE_APIS_AVAILABLE_SLOT,
        storage_apis_available_value.into(),
    );
    if storage_apis_available {
        GlobalCachesAccessorDeclaration::default()
            .initialize(scope, global)
            .map_err(|error| anyhow!("failed to initialize window caches accessor: {error}"))?;
    }
    Ok(())
}

pub(crate) fn set_window_navigator_identity(
    scope: &mut v8::PinScope<'_, '_>,
    identity: &BrowserIdentityProfile,
) -> Result<()> {
    let global = scope.get_current_context().global(scope);
    bind_window_navigator_identity_seed(scope, global, identity)
}

pub(crate) fn bind_window_navigator_identity_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    identity: &BrowserIdentityProfile,
) -> Result<()> {
    let relevant_context = window
        .get_creation_context(scope)
        .ok_or_else(|| anyhow!("Navigator seed target has no creation context"))?;
    if relevant_context != scope.get_current_context() {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let target_window = relevant_context.global(target_scope);
        return bind_window_navigator_identity_seed_in_current_realm(
            target_scope,
            target_window,
            identity,
        );
    }
    bind_window_navigator_identity_seed_in_current_realm(scope, window, identity)
}

fn bind_window_navigator_identity_seed_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    identity: &BrowserIdentityProfile,
) -> Result<()> {
    set_navigator_identity_profile(scope, window, identity);
    let user_agent_value = v8_string(scope, identity.user_agent())
        .ok_or_else(|| anyhow!("failed to allocate navigator.userAgent string"))?;
    set_private_value(
        scope,
        window,
        WINDOW_NAVIGATOR_USER_AGENT_SLOT,
        user_agent_value.into(),
    );
    let accept_language_value = v8_string(scope, identity.accept_language())
        .ok_or_else(|| anyhow!("failed to allocate Navigator Accept-Language string"))?;
    set_private_value(
        scope,
        window,
        WINDOW_NAVIGATOR_ACCEPT_LANGUAGE_SLOT,
        accept_language_value.into(),
    );
    let Some(navigator) = get_private_value(scope, window, WINDOW_NAVIGATOR_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Ok(());
    };
    update_window_navigator_identity(scope, navigator, identity)
}
