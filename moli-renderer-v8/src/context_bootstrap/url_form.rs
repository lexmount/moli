//! Object URLs are registered by the embedding storage owner; ordinary URL
//! parsing and all URLSearchParams bindings live in moli-webapi.
use super::exposed_interfaces::TemplateBuildProfile;
use super::*;
use crate::util::{throw_type_error, v8_string};
use crate::{blob, web_api_interfaces, webidl};
use moli_webapi_declare::WebApiFunctionTemplate;
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URL.revokeObjectURL")]
struct UrlRevokeObjectUrlArgs {
    #[webidl(required)]
    url: String,
}

fn url_create_object_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(storage_key) = current_object_url_storage_key(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(object) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(
            scope,
            "Failed to execute 'createObjectURL' on 'URL': parameter 1 is not of type 'Blob'.",
        );
        return;
    };
    #[cfg(all(
        feature = "media-ext",
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    if let Some(source) = super::media_source_playback::media_source_object(scope, object)
        && let Some(host) = context_host_ptr_from_global_bridge(scope)
        && let Some(url) =
            unsafe { &mut *host }.create_media_source_url(scope, source, storage_key.clone())
        && let Some(value) = v8_string(scope, &url)
    {
        rv.set(value.into());
        return;
    }
    let Some(url) = blob::create_object_url_for_object(scope, object, storage_key) else {
        throw_type_error(
            scope,
            "Failed to execute 'createObjectURL' on 'URL': parameter 1 is not of type 'Blob'.",
        );
        return;
    };
    if let Some(value) = v8_string(scope, &url) {
        rv.set(value.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

fn current_object_url_storage_key(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<moli_storage_key::MoliStorageKey> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        // A child resource task can invoke callbacks in a different Window.
        // Object URL storage follows the executing realm's settings object.
        let identity = host.current_runtime_window_execution_context_identity(scope)?;
        return host
            .storage_context_for_window_execution_context_identity(identity)
            .map(|context| context.storage_key().clone());
    }
    crate::worker::worker_storage_key(scope)
}

fn url_revoke_object_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<UrlRevokeObjectUrlArgs>(scope, &args) else {
        return;
    };
    if parsed.url.starts_with("blob:")
        && let Some(storage_key) = current_object_url_storage_key(scope)
    {
        #[cfg(all(
            feature = "media-ext",
            any(target_os = "linux", target_os = "macos"),
            target_pointer_width = "64"
        ))]
        if let Some(host) = context_host_ptr_from_global_bridge(scope) {
            unsafe { &mut *host }.revoke_media_source_url(&parsed.url, &storage_key);
        }
        blob::revoke_object_url(scope, &parsed.url, storage_key);
    }
    rv.set_undefined();
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::URL, enumerable)]
struct UrlObjectUrlStaticsDeclaration {
    #[webapi(
        static_method = "createObjectURL",
        length = 1,
        callback = url_create_object_url_callback
    )]
    create_object_url: (),

    #[webapi(
        static_method = "revokeObjectURL",
        length = 1,
        callback = url_revoke_object_url_callback
    )]
    revoke_object_url: (),
}

pub(super) fn build_url_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    profile: TemplateBuildProfile,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = moli_webapi::url_api::build_url_constructor_template(scope);
    if matches!(
        profile,
        TemplateBuildProfile::Window
            | TemplateBuildProfile::DedicatedWorker
            | TemplateBuildProfile::SharedWorker
    ) {
        UrlObjectUrlStaticsDeclaration::initialize_template(scope, template);
    }
    template
}
