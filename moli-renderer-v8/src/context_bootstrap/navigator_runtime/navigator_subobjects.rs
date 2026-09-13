use super::navigator::build_lazy_navigator_subobject_in_current_realm;
use crate::util::{get_private_value, set_private_value, v8str};
use anyhow::{Result, anyhow};

const MATERIALIZING_SLOT: &str = "__moliNavigatorSubobjectMaterializing";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavigatorSubobject {
    Languages,
    MimeTypes,
    Plugins,
    Connection,
    UserAgentData,
    Permissions,
    Storage,
    WebkitTemporaryStorage,
    WebkitPersistentStorage,
    MediaDevices,
    ServiceWorker,
    Clipboard,
    UserActivation,
    StorageBuckets,
    Geolocation,
    MediaCapabilities,
    Scheduling,
}

impl NavigatorSubobject {
    #[cfg(test)]
    pub(super) const ALL: [Self; 17] = [
        Self::Languages,
        Self::MimeTypes,
        Self::Plugins,
        Self::Connection,
        Self::UserAgentData,
        Self::Permissions,
        Self::Storage,
        Self::WebkitTemporaryStorage,
        Self::WebkitPersistentStorage,
        Self::MediaDevices,
        Self::ServiceWorker,
        Self::Clipboard,
        Self::UserActivation,
        Self::StorageBuckets,
        Self::Geolocation,
        Self::MediaCapabilities,
        Self::Scheduling,
    ];

    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "languages" => Some(Self::Languages),
            "mimeTypes" => Some(Self::MimeTypes),
            "plugins" => Some(Self::Plugins),
            "connection" => Some(Self::Connection),
            "userAgentData" => Some(Self::UserAgentData),
            "permissions" => Some(Self::Permissions),
            "storage" => Some(Self::Storage),
            "webkitTemporaryStorage" => Some(Self::WebkitTemporaryStorage),
            "webkitPersistentStorage" => Some(Self::WebkitPersistentStorage),
            "mediaDevices" => Some(Self::MediaDevices),
            "serviceWorker" => Some(Self::ServiceWorker),
            "clipboard" => Some(Self::Clipboard),
            "userActivation" => Some(Self::UserActivation),
            "storageBuckets" => Some(Self::StorageBuckets),
            "geolocation" => Some(Self::Geolocation),
            "mediaCapabilities" => Some(Self::MediaCapabilities),
            "scheduling" => Some(Self::Scheduling),
            _ => None,
        }
    }

    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Languages => "languages",
            Self::MimeTypes => "mimeTypes",
            Self::Plugins => "plugins",
            Self::Connection => "connection",
            Self::UserAgentData => "userAgentData",
            Self::Permissions => "permissions",
            Self::Storage => "storage",
            Self::WebkitTemporaryStorage => "webkitTemporaryStorage",
            Self::WebkitPersistentStorage => "webkitPersistentStorage",
            Self::MediaDevices => "mediaDevices",
            Self::ServiceWorker => "serviceWorker",
            Self::Clipboard => "clipboard",
            Self::UserActivation => "userActivation",
            Self::StorageBuckets => "storageBuckets",
            Self::Geolocation => "geolocation",
            Self::MediaCapabilities => "mediaCapabilities",
            Self::Scheduling => "scheduling",
        }
    }
}

pub(super) fn ensure_navigator_subobject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
    subobject: NavigatorSubobject,
) -> Result<v8::Local<'s, v8::Value>> {
    if let Some(value) = cached_value(scope, backing, subobject) {
        return Ok(value);
    }
    let relevant_context = backing
        .get_creation_context(scope)
        .ok_or_else(|| anyhow!("Navigator backing has no creation context"))?;
    if relevant_context == scope.get_current_context() {
        return build_and_cache_in_current_realm(scope, backing, subobject);
    }

    let backing = v8::Global::new(scope, backing);
    let value = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let target_backing = v8::Local::new(target_scope, &backing);
        let value = build_and_cache_in_current_realm(target_scope, target_backing, subobject)?;
        v8::Global::new(target_scope, value)
    };
    Ok(v8::Local::new(scope, &value))
}

fn build_and_cache_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
    subobject: NavigatorSubobject,
) -> Result<v8::Local<'s, v8::Value>> {
    if let Some(active) = get_private_value(scope, backing, MATERIALIZING_SLOT) {
        let active = active
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(anyhow!(
            "reentrant navigator.{} materialization while building navigator.{active}",
            subobject.key()
        ));
    }
    set_private_value(
        scope,
        backing,
        MATERIALIZING_SLOT,
        v8str(scope, subobject.key()).into(),
    );
    let result = build_lazy_navigator_subobject_in_current_realm(scope, backing, subobject);
    set_private_value(
        scope,
        backing,
        MATERIALIZING_SLOT,
        v8::undefined(scope).into(),
    );
    let value = result?;
    if backing.set(scope, v8str(scope, subobject.key()).into(), value) != Some(true) {
        return Err(anyhow!(
            "failed to cache navigator.{} SameObject",
            subobject.key()
        ));
    }
    Ok(value)
}

fn cached_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
    subobject: NavigatorSubobject,
) -> Option<v8::Local<'s, v8::Value>> {
    backing
        .get(scope, v8str(scope, subobject.key()).into())
        .filter(|value| !value.is_undefined())
}
