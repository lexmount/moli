use moli_storage_service::StorageBucketLocator;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use super::opfs::resolve_opfs_root_with_handle_access;
use crate::native_bridge::OwnerDispatchScope;
use crate::opfs_owner_tasks::OpfsHandleAccessContext;
use crate::util::{context_host_ptr_from_global_bridge, get_private_value, v8_string};
use crate::webidl;

const STORAGE_ACCESS_HANDLE_BRAND_SLOT: &str = "__moliStorageAccessHandleBrand";
const STORAGE_ACCESS_HANDLE_GET_DIRECTORY_SLOT: &str = "__moliStorageAccessHandleGetDirectory";
const STORAGE_ACCESS_HANDLE_STORAGE_KEY_SLOT: &str = "__moliStorageAccessHandleStorageKey";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StorageAccessPermissionState {
    Granted,
    Denied,
    Prompt,
}

impl StorageAccessPermissionState {
    fn from_label(label: &str) -> Self {
        match label {
            "granted" => Self::Granted,
            "denied" => Self::Denied,
            _ => Self::Prompt,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StorageAccessRequestPolicy {
    outermost: bool,
    third_party_partitioned: bool,
    permission: StorageAccessPermissionState,
    transient_user_activation: bool,
}

impl StorageAccessRequestPolicy {
    fn allows_request(self) -> bool {
        if self.outermost {
            return true;
        }
        match self.permission {
            StorageAccessPermissionState::Granted => true,
            StorageAccessPermissionState::Denied => false,
            StorageAccessPermissionState::Prompt => {
                !self.third_party_partitioned || self.transient_user_activation
            }
        }
    }
}

struct StorageAccessRequestContext {
    ambient_storage_key: String,
    policy: StorageAccessRequestPolicy,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "StorageAccessTypes")]
struct StorageAccessTypes {
    #[webidl(default = false)]
    all: bool,
    #[webidl(default = false)]
    cookies: bool,
    #[webidl(name = "sessionStorage", default = false)]
    session_storage: bool,
    #[webidl(name = "localStorage", default = false)]
    local_storage: bool,
    #[webidl(name = "indexedDB", default = false)]
    indexed_db: bool,
    #[webidl(default = false)]
    locks: bool,
    #[webidl(default = false)]
    caches: bool,
    #[webidl(name = "getDirectory", default = false)]
    get_directory: bool,
    #[webidl(default = false)]
    estimate: bool,
    #[webidl(name = "createObjectURL", default = false)]
    create_object_url: bool,
    #[webidl(name = "revokeObjectURL", default = false)]
    revoke_object_url: bool,
    #[webidl(name = "BroadcastChannel", default = false)]
    broadcast_channel: bool,
    #[webidl(name = "SharedWorker", default = false)]
    shared_worker: bool,
}

impl StorageAccessTypes {
    fn requests_any_access(&self) -> bool {
        self.all
            || self.cookies
            || self.session_storage
            || self.local_storage
            || self.indexed_db
            || self.locks
            || self.caches
            || self.get_directory
            || self.estimate
            || self.create_object_url
            || self.revoke_object_url
            || self.broadcast_channel
            || self.shared_worker
    }

    fn requests_get_directory(&self) -> bool {
        self.all || self.get_directory
    }
}

#[derive(WebApiObject)]
#[webapi(interface = "StorageAccessHandle", require_prototype)]
struct StorageAccessHandleObjectDeclaration {
    #[webapi(
        slot,
        name = STORAGE_ACCESS_HANDLE_BRAND_SLOT,
        constructor_default = true
    )]
    brand: bool,
    #[webapi(slot = STORAGE_ACCESS_HANDLE_GET_DIRECTORY_SLOT)]
    get_directory: bool,
    #[webapi(slot = STORAGE_ACCESS_HANDLE_STORAGE_KEY_SLOT)]
    storage_key: String,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "StorageAccessHandle", enumerable)]
struct StorageAccessHandlePrototypeDeclaration {
    #[webapi(
        method,
        length = 0,
        callback = storage_access_handle_get_directory_callback
    )]
    get_directory: (),
}

pub(in crate::context_bootstrap) fn install_storage_access_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "StorageAccessHandle" {
        StorageAccessHandlePrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

/// Implements the options overload of `Document.requestStorageAccess()`.
///
/// The no-argument cookie overload remains owned by `Document`; this function
/// is called only when script supplied a `StorageAccessTypes` dictionary.
pub(crate) fn request_storage_access_with_types<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    let types = match webidl::parse_dictionary::<StorageAccessTypes>(
        scope,
        args.get(0),
        webidl::Context::argument("Document.requestStorageAccess", 1),
    ) {
        Ok(Some(types)) => types,
        Ok(None) => StorageAccessTypes::default(),
        Err(error) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    if !types.requests_any_access() {
        reject_dom_exception(
            scope,
            resolver,
            "SecurityError",
            "You must request access for at least one storage/communication medium.",
        );
        return;
    }
    let Some(request_context) = current_window_storage_access_request_context(scope) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The document storage context is unavailable.",
        );
        return;
    };
    let Some(storage_key) = unpartitioned_storage_key(&request_context.ambient_storage_key) else {
        reject_dom_exception(
            scope,
            resolver,
            "NotAllowedError",
            "Storage access is unavailable for an opaque origin.",
        );
        return;
    };
    if !request_context.policy.allows_request() {
        reject_dom_exception(
            scope,
            resolver,
            "NotAllowedError",
            "Storage access requires permission and transient user activation.",
        );
        return;
    }
    let handle =
        StorageAccessHandleObjectDeclaration::new(types.requests_get_directory(), storage_key)
            .bind(scope);
    match handle {
        Ok(handle) => {
            let _ = resolver.resolve(scope, handle.into());
        }
        Err(_) => reject_dom_exception(
            scope,
            resolver,
            "UnknownError",
            "Failed to create a StorageAccessHandle wrapper.",
        ),
    }
}

fn current_window_storage_access_request_context(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<StorageAccessRequestContext> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    // SAFETY: the global bridge owns the host for each live Window realm in
    // this isolate. Identity, storage context, permission state, and protocol
    // activation are copied while the current V8 context is entered.
    let host = unsafe { &mut *host_ptr };
    let identity = host.current_runtime_window_execution_context_identity(scope)?;
    let ambient_storage_key = host
        .storage_context_for_window_execution_context_identity(identity)?
        .storage_key()
        .serialized_storage_key();
    let storage_key = moli_storage_key::deserialize_serialized_storage_key(&ambient_storage_key)?;
    let embedding_origin = moli_url::origin_ascii_serialization(host.document_url());
    let permission = StorageAccessPermissionState::from_label(host.permission_state_for_origins(
        "storage-access",
        storage_key.origin(),
        &embedding_origin,
    ));
    let outermost = matches!(identity.dispatch_scope(), OwnerDispatchScope::Top);
    Some(StorageAccessRequestContext {
        ambient_storage_key,
        policy: StorageAccessRequestPolicy {
            outermost,
            third_party_partitioned: storage_key.is_third_party_partitioned(),
            permission,
            transient_user_activation: host.transient_user_activation(),
        },
    })
}

fn unpartitioned_storage_key(ambient_storage_key: &str) -> Option<String> {
    let storage_key = moli_storage_key::deserialize_serialized_storage_key(ambient_storage_key)?;
    if storage_key.origin() == "null" {
        return None;
    }
    let origin = url::Url::parse(storage_key.origin()).ok()?;
    Some(
        moli_storage_key::MoliStorageKey::first_party_from_url(&origin, None)
            .serialized_storage_key(),
    )
}

fn storage_access_handle_get_directory_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    if !get_private_value(scope, args.this(), STORAGE_ACCESS_HANDLE_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        reject_type_error(scope, resolver, "Illegal invocation");
        return;
    }
    if !get_private_value(scope, args.this(), STORAGE_ACCESS_HANDLE_GET_DIRECTORY_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        reject_dom_exception(
            scope,
            resolver,
            "SecurityError",
            "Origin Private File System not requested when storage access handle was initialized.",
        );
        return;
    }
    let Some(storage_key) =
        get_private_value(scope, args.this(), STORAGE_ACCESS_HANDLE_STORAGE_KEY_SLOT)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
    else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The StorageAccessHandle storage context is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The StorageAccessHandle owner is unavailable.",
        );
        return;
    };
    let Some(creation_context) = args.this().get_creation_context(scope) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The StorageAccessHandle owner realm is unavailable.",
        );
        return;
    };
    // SAFETY: the global bridge owns the host for every Window realm in this
    // isolate. The lookup returns a copyable identity and verifies it again
    // when the OPFS task is accepted and settled.
    let host = unsafe { &*host_ptr };
    let Some(identity) =
        host.window_execution_context_identity_for_v8_context(scope, creation_context)
    else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The StorageAccessHandle owner realm is no longer active.",
        );
        return;
    };
    if !host.window_execution_context_identity_is_current(identity) {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "The StorageAccessHandle owner realm is no longer active.",
        );
        return;
    }
    let locator = StorageBucketLocator::default_bucket(storage_key.clone());
    resolve_opfs_root_with_handle_access(
        scope,
        resolver,
        locator,
        OpfsHandleAccessContext::window(identity, storage_key),
    );
}

fn reject_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
) {
    let error = v8_string(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, error);
}

fn reject_dom_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    name: &str,
    message: &str,
) {
    let error = crate::context_bootstrap::new_dom_exception_value(scope, message, name);
    let _ = resolver.reject(scope, error);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(
        outermost: bool,
        third_party_partitioned: bool,
        permission: StorageAccessPermissionState,
        transient_user_activation: bool,
    ) -> StorageAccessRequestPolicy {
        StorageAccessRequestPolicy {
            outermost,
            third_party_partitioned,
            permission,
            transient_user_activation,
        }
    }

    #[test]
    fn storage_access_key_rehomes_partitioned_origin_to_first_party() {
        let partitioned =
            moli_storage_key::partitioned_storage_key("https://embedded.test", "https://top.test");
        let expected = moli_storage_key::partitioned_storage_key(
            "https://embedded.test",
            "https://embedded.test",
        );
        assert_eq!(unpartitioned_storage_key(&partitioned), Some(expected));
    }

    #[test]
    fn storage_access_key_rejects_opaque_origins() {
        let opaque = moli_storage_key::MoliStorageKey::new(
            "null".to_owned(),
            "https://top.test".to_owned(),
            Some(moli_storage_key::OpaqueOriginNonce::new(7)),
            moli_storage_key::StoragePartitionRelation::Unknown,
        )
        .serialized_storage_key();
        assert_eq!(unpartitioned_storage_key(&opaque), None);
    }

    #[test]
    fn outermost_storage_access_does_not_require_permission_or_activation() {
        assert!(policy(true, false, StorageAccessPermissionState::Denied, false).allows_request());
    }

    #[test]
    fn same_site_and_aba_storage_access_can_use_autogrant() {
        let aba_key = moli_storage_key::MoliStorageKey::first_party_from_url(
            &url::Url::parse("https://top.test/inner").expect("ABA URL"),
            None,
        )
        .with_cross_site_ancestor();
        assert!(!aba_key.is_third_party_partitioned());
        assert!(aba_key.has_cross_site_ancestor());
        assert!(
            policy(
                false,
                aba_key.is_third_party_partitioned(),
                StorageAccessPermissionState::Prompt,
                false,
            )
            .allows_request()
        );
    }

    #[test]
    fn third_party_prompt_requires_transient_user_activation() {
        assert!(!policy(false, true, StorageAccessPermissionState::Prompt, false).allows_request());
        assert!(policy(false, true, StorageAccessPermissionState::Prompt, true).allows_request());
    }

    #[test]
    fn existing_storage_access_grant_does_not_require_activation() {
        assert!(
            policy(false, true, StorageAccessPermissionState::Granted, false,).allows_request()
        );
    }

    #[test]
    fn denied_storage_access_rejects_even_with_activation() {
        assert!(!policy(false, true, StorageAccessPermissionState::Denied, true).allows_request());
    }
}
