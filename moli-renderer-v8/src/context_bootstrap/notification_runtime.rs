use super::{
    SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, SIMPLE_EVENT_TARGET_SLOT, global_constructor_object,
    simple_event_target_add_event_listener_callback, simple_event_target_dispatch_event_callback,
    simple_event_target_remove_event_listener_callback, structured_clone_value_for_storage,
};
use crate::web_api_interfaces;
use crate::{
    runtime::{
        ServiceWorkerNotificationMetadata, ServiceWorkerNotificationSnapshot,
        ServiceWorkerRegistrationId,
    },
    service_worker_runtime::ServiceWorkerNotificationAction,
    structured_clone::V8StructuredClonePayload,
    util::{
        context_host_ptr_from_global_bridge, get_private_value, set_private_value,
        throw_type_error, v8_string, v8str,
    },
    webidl,
};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiFunctionTemplate, WebApiObject};
use url::Url;

const NOTIFICATION_TITLE_SLOT: &str = "__lmNotificationTitle";
const NOTIFICATION_DATA_SLOT: &str = "__lmNotificationData";
const NOTIFICATION_TAG_SLOT: &str = "__lmNotificationTag";
const NOTIFICATION_DIR_SLOT: &str = "__lmNotificationDir";
const NOTIFICATION_LANG_SLOT: &str = "__lmNotificationLang";
const NOTIFICATION_BODY_SLOT: &str = "__lmNotificationBody";
const NOTIFICATION_ICON_SLOT: &str = "__lmNotificationIcon";
const NOTIFICATION_IMAGE_SLOT: &str = "__lmNotificationImage";
const NOTIFICATION_BADGE_SLOT: &str = "__lmNotificationBadge";
const NOTIFICATION_VIBRATE_SLOT: &str = "__lmNotificationVibrate";
const NOTIFICATION_TIMESTAMP_SLOT: &str = "__lmNotificationTimestamp";
const NOTIFICATION_RENOTIFY_SLOT: &str = "__lmNotificationRenotify";
const NOTIFICATION_SILENT_SLOT: &str = "__lmNotificationSilent";
const NOTIFICATION_REQUIRE_INTERACTION_SLOT: &str = "__lmNotificationRequireInteraction";
const NOTIFICATION_ACTIONS_SLOT: &str = "__lmNotificationActions";
const NOTIFICATION_RECORD_ID_SLOT: &str = "__lmNotificationRecordId";
const NOTIFICATION_RECORD_REGISTRATION_ID_SLOT: &str = "__lmNotificationRecordRegistrationId";
const NOTIFICATION_LISTENERS_SLOT: &str = "__lmNotificationListeners";
const NOTIFICATION_MAX_ACTIONS: usize = 2;

pub(crate) struct NotificationOptionsPayload {
    pub(crate) data: V8StructuredClonePayload,
    pub(crate) tag: String,
    pub(crate) metadata: ServiceWorkerNotificationMetadata,
    pub(crate) actions: Vec<ServiceWorkerNotificationAction>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Notification)]
struct NotificationObjectDeclaration<'scope> {
    #[webapi(slot = NOTIFICATION_TITLE_SLOT)]
    title: String,

    #[webapi(slot = NOTIFICATION_DATA_SLOT)]
    data: v8::Local<'scope, v8::Value>,

    #[webapi(slot = NOTIFICATION_TAG_SLOT)]
    tag: String,

    #[webapi(slot = NOTIFICATION_DIR_SLOT)]
    dir: String,

    #[webapi(slot = NOTIFICATION_LANG_SLOT)]
    lang: String,

    #[webapi(slot = NOTIFICATION_BODY_SLOT)]
    body: String,

    #[webapi(slot = NOTIFICATION_ICON_SLOT)]
    icon: String,

    #[webapi(slot = NOTIFICATION_IMAGE_SLOT)]
    image: String,

    #[webapi(slot = NOTIFICATION_BADGE_SLOT)]
    badge: String,

    #[webapi(slot = NOTIFICATION_VIBRATE_SLOT)]
    vibrate: v8::Local<'scope, v8::Value>,

    #[webapi(slot = NOTIFICATION_TIMESTAMP_SLOT)]
    timestamp: f64,

    #[webapi(slot = NOTIFICATION_RENOTIFY_SLOT)]
    renotify: bool,

    #[webapi(slot = NOTIFICATION_SILENT_SLOT)]
    silent: v8::Local<'scope, v8::Value>,

    #[webapi(slot = NOTIFICATION_REQUIRE_INTERACTION_SLOT)]
    require_interaction: bool,

    #[webapi(slot = NOTIFICATION_ACTIONS_SLOT)]
    actions: v8::Local<'scope, v8::Value>,

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = NOTIFICATION_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(data_property = "onclick", init = "null")]
    onclick: (),

    #[webapi(data_property = "onerror", init = "null")]
    onerror: (),

    #[webapi(data_property = "onshow", init = "null")]
    onshow: (),

    #[webapi(data_property = "onclose", init = "null")]
    onclose: (),

    #[webapi(
        method,
        enumerable,
        callback = notification_event_target_add_event_listener_callback
    )]
    add_event_listener: (),

    #[webapi(
        method,
        enumerable,
        callback = notification_event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),

    #[webapi(
        method,
        enumerable,
        callback = notification_event_target_dispatch_event_callback
    )]
    dispatch_event: (),

    #[webapi(method, enumerable, callback = notification_close_callback)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Notification)]
struct NotificationPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = notification_title_getter_callback,
        enumerable
    )]
    title: (),

    #[webapi(accessor_property, getter = notification_data_getter_callback, enumerable)]
    data: (),

    #[webapi(accessor_property, getter = notification_tag_getter_callback, enumerable)]
    tag: (),

    #[webapi(accessor_property, getter = notification_dir_getter_callback, enumerable)]
    dir: (),

    #[webapi(accessor_property, getter = notification_lang_getter_callback, enumerable)]
    lang: (),

    #[webapi(accessor_property, getter = notification_body_getter_callback, enumerable)]
    body: (),

    #[webapi(accessor_property, getter = notification_icon_getter_callback, enumerable)]
    icon: (),

    #[webapi(accessor_property, getter = notification_image_getter_callback, enumerable)]
    image: (),

    #[webapi(accessor_property, getter = notification_badge_getter_callback, enumerable)]
    badge: (),

    #[webapi(accessor_property, getter = notification_vibrate_getter_callback, enumerable)]
    vibrate: (),

    #[webapi(accessor_property, getter = notification_timestamp_getter_callback, enumerable)]
    timestamp: (),

    #[webapi(accessor_property, getter = notification_renotify_getter_callback, enumerable)]
    renotify: (),

    #[webapi(accessor_property, getter = notification_silent_getter_callback, enumerable)]
    silent: (),

    #[webapi(
        accessor_property,
        getter = notification_require_interaction_getter_callback,
        enumerable
    )]
    require_interaction: (),

    #[webapi(accessor_property, getter = notification_actions_getter_callback, enumerable)]
    actions: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Notification, enumerable)]
struct NotificationConstructorDeclaration {
    #[webapi(static_method, length = 0, callback = notification_request_permission_callback)]
    request_permission: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Notification)]
struct NotificationConstructorConstantsDeclaration {
    #[webapi(constant = "maxActions", value = NOTIFICATION_MAX_ACTIONS)]
    max_actions: (),
}

#[derive(Default, WebApiObject)]
#[webapi(record)]
struct NotificationConstructorPermissionDeclaration {
    #[webapi(
        accessor_property,
        getter = notification_permission_getter_callback
    )]
    permission: (),
}

pub(in crate::context_bootstrap) fn notification_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Notification': Please use the 'new' operator.",
        );
        return;
    }
    if args.length() < 1 {
        throw_type_error(
            scope,
            "Failed to construct 'Notification': 1 argument required, but only 0 present.",
        );
        return;
    }

    let Some(title) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("Notification", 1),
        "Failed to construct 'Notification': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(options) = notification_options(scope, args.get(1)) else {
        return;
    };

    let notification = args.this();
    initialize_notification_object(
        scope,
        notification,
        title.0,
        options.tag,
        options.metadata,
        options.data,
        options.actions,
        None,
    );
    rv.set(notification.into());
}

pub(in crate::context_bootstrap) fn install_notification_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    NotificationConstructorDeclaration::initialize_template(scope, template);
    NotificationConstructorConstantsDeclaration::initialize_template(scope, template);
    NotificationPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn install_notification_realm_bindings(
    scope: &mut v8::PinScope<'_, '_>,
) {
    if let Some(constructor) = global_constructor_object(scope, "Notification") {
        let _ =
            NotificationConstructorPermissionDeclaration::default().initialize(scope, constructor);
    }
}

fn notification_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
) -> Option<NotificationObjectOptions<'s>> {
    let options =
        match webidl::dictionary_value(options, webidl::Context::argument("Notification", 2)) {
            Ok(Some(options)) => options,
            Ok(None) => {
                return Some(NotificationObjectOptions {
                    data: v8::null(scope).into(),
                    tag: String::new(),
                    metadata: default_notification_metadata(),
                    actions: Vec::new(),
                });
            }
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
    let data = match webidl::property_result(
        scope,
        options,
        "data",
        webidl::Context::member("NotificationOptions", "data"),
    ) {
        Ok(Some(data)) if !data.is_undefined() => data,
        Ok(_) => {
            return Some(NotificationObjectOptions {
                data: v8::null(scope).into(),
                tag: notification_options_tag(scope, options, "NotificationOptions")?,
                metadata: notification_options_metadata(scope, options)?,
                actions: notification_constructor_actions(scope, options)?,
            });
        }
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let tag = notification_options_tag(scope, options, "NotificationOptions")?;
    let metadata = notification_options_metadata(scope, options)?;
    let actions = notification_constructor_actions(scope, options)?;
    structured_clone_value_for_storage(scope, data).map(|data| NotificationObjectOptions {
        data,
        tag,
        metadata,
        actions,
    })
}

struct NotificationObjectOptions<'scope> {
    data: v8::Local<'scope, v8::Value>,
    tag: String,
    metadata: ServiceWorkerNotificationMetadata,
    actions: Vec<ServiceWorkerNotificationAction>,
}

pub(crate) fn notification_options_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
) -> Option<NotificationOptionsPayload> {
    let options =
        match webidl::dictionary_value(options, webidl::Context::argument("Notification", 2)) {
            Ok(Some(options)) => options,
            Ok(None) => {
                let null_value: v8::Local<'_, v8::Value> = v8::null(scope).into();
                let data = crate::context_bootstrap::structured_serialize_value_for_storage(
                    scope, null_value,
                )?;
                return Some(NotificationOptionsPayload {
                    data,
                    tag: String::new(),
                    metadata: default_notification_metadata(),
                    actions: Vec::new(),
                });
            }
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
    let tag = notification_options_tag(scope, options, "NotificationOptions")?;
    let metadata = notification_options_metadata(scope, options)?;
    let actions = notification_options_actions(scope, options)?;
    let data = match webidl::property_result(
        scope,
        options,
        "data",
        webidl::Context::member("NotificationOptions", "data"),
    ) {
        Ok(Some(data)) if !data.is_undefined() => data,
        Ok(_) => {
            let null_value: v8::Local<'_, v8::Value> = v8::null(scope).into();
            let data = crate::context_bootstrap::structured_serialize_value_for_storage(
                scope, null_value,
            )?;
            return Some(NotificationOptionsPayload {
                data,
                tag,
                metadata,
                actions,
            });
        }
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    crate::context_bootstrap::structured_serialize_value_for_storage(scope, data).map(|data| {
        NotificationOptionsPayload {
            data,
            tag,
            metadata,
            actions,
        }
    })
}

pub(crate) fn notification_get_options_tag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
) -> Option<Option<String>> {
    let options = match webidl::dictionary_value(
        options,
        webidl::Context::argument("ServiceWorkerRegistration.getNotifications", 1),
    ) {
        Ok(Some(options)) => options,
        Ok(None) => return Some(None),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let tag = notification_options_tag(scope, options, "GetNotificationOptions")?;
    if tag.is_empty() {
        Some(None)
    } else {
        Some(Some(tag))
    }
}

fn notification_options_tag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    context_name: &'static str,
) -> Option<String> {
    let tag = match webidl::property_result(
        scope,
        options,
        "tag",
        webidl::Context::member(context_name, "tag"),
    ) {
        Ok(Some(tag)) if !tag.is_undefined() => tag,
        Ok(_) => return Some(String::new()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    match webidl::convert::<webidl::DomString>(
        scope,
        tag,
        webidl::Context::member(context_name, "tag"),
    ) {
        Ok(tag) => Some(tag.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn default_notification_metadata() -> ServiceWorkerNotificationMetadata {
    ServiceWorkerNotificationMetadata {
        dir: "auto".to_owned(),
        ..ServiceWorkerNotificationMetadata::default()
    }
}

fn notification_options_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerNotificationMetadata> {
    Some(ServiceWorkerNotificationMetadata {
        dir: notification_options_dir(scope, options)?,
        lang: notification_options_string(scope, options, "lang")?,
        body: notification_options_string(scope, options, "body")?,
        icon: notification_options_string(scope, options, "icon")?,
        image: notification_options_string(scope, options, "image")?,
        badge: notification_options_string(scope, options, "badge")?,
        vibrate: notification_options_vibrate(scope, options)?,
        timestamp: notification_options_timestamp(scope, options)?,
        renotify: notification_options_bool(scope, options, "renotify")?,
        silent: notification_options_silent(scope, options)?,
        require_interaction: notification_options_bool(scope, options, "requireInteraction")?,
    })
}

fn notification_options_dir<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let dir = notification_options_string_with_default(scope, options, "dir", "auto")?;
    match dir.as_str() {
        "auto" | "ltr" | "rtl" => Some(dir),
        _ => {
            throw_type_error(
                scope,
                "Failed to read the 'dir' property from 'NotificationOptions': the provided value is not a valid enum value.",
            );
            None
        }
    }
}

fn notification_options_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<String> {
    notification_options_string_with_default(scope, options, name, "")
}

fn notification_options_string_with_default<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    name: &'static str,
    default_value: &'static str,
) -> Option<String> {
    let value = match webidl::property_result(
        scope,
        options,
        name,
        webidl::Context::member("NotificationOptions", name),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(default_value.to_owned()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    match webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("NotificationOptions", name),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn notification_options_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<bool> {
    let value = match webidl::property_result(
        scope,
        options,
        name,
        webidl::Context::member("NotificationOptions", name),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(false),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    match webidl::convert::<webidl::Boolean>(
        scope,
        value,
        webidl::Context::member("NotificationOptions", name),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn notification_options_silent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
) -> Option<Option<bool>> {
    let value = match webidl::property_result(
        scope,
        options,
        "silent",
        webidl::Context::member("NotificationOptions", "silent"),
    ) {
        Ok(Some(value)) if !value.is_undefined() && !value.is_null() => value,
        Ok(_) => return Some(None),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    match webidl::convert::<webidl::Boolean>(
        scope,
        value,
        webidl::Context::member("NotificationOptions", "silent"),
    ) {
        Ok(value) => Some(Some(value.0)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn notification_options_timestamp<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
) -> Option<Option<u64>> {
    let value = match webidl::property_result(
        scope,
        options,
        "timestamp",
        webidl::Context::member("NotificationOptions", "timestamp"),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(None),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    match webidl::convert::<webidl::UnsignedLongLong>(
        scope,
        value,
        webidl::Context::member("NotificationOptions", "timestamp"),
    ) {
        Ok(value) => Some(Some(value.0)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn notification_options_vibrate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
) -> Option<Vec<u32>> {
    let value = match webidl::property_result(
        scope,
        options,
        "vibrate",
        webidl::Context::member("NotificationOptions", "vibrate"),
    ) {
        Ok(Some(value)) if !value.is_undefined() && !value.is_null() => value,
        Ok(_) => return Some(Vec::new()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
        let mut pattern = Vec::with_capacity(array.length() as usize);
        for index in 0..array.length() {
            let Some(item) = array.get_index(scope, index) else {
                continue;
            };
            let value = match webidl::convert::<webidl::UnsignedLong>(
                scope,
                item,
                webidl::Context::member("NotificationOptions", "vibrate"),
            ) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return None;
                }
            };
            pattern.push(value);
        }
        return Some(pattern);
    }
    match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member("NotificationOptions", "vibrate"),
    ) {
        Ok(value) => Some(vec![value.0]),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn notification_constructor_actions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
) -> Option<Vec<ServiceWorkerNotificationAction>> {
    let actions = match webidl::property_result(
        scope,
        options,
        "actions",
        webidl::Context::member("NotificationOptions", "actions"),
    ) {
        Ok(Some(actions)) if !actions.is_undefined() => actions,
        Ok(_) => return Some(Vec::new()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    if actions.is_null_or_undefined() {
        return Some(Vec::new());
    }
    throw_type_error(
        scope,
        "Failed to construct 'Notification': non-persistent notifications do not support actions.",
    );
    None
}

fn notification_options_actions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Object>,
) -> Option<Vec<ServiceWorkerNotificationAction>> {
    let actions = match webidl::property_result(
        scope,
        options,
        "actions",
        webidl::Context::member("NotificationOptions", "actions"),
    ) {
        Ok(Some(actions)) if !actions.is_undefined() && !actions.is_null() => actions,
        Ok(_) => return Some(Vec::new()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Some(actions) = v8::Local::<v8::Array>::try_from(actions).ok() else {
        throw_type_error(
            scope,
            "Failed to read the 'actions' property from 'NotificationOptions': the provided value is not an array.",
        );
        return None;
    };
    let mut parsed = Vec::new();
    for index in 0..actions.length().min(NOTIFICATION_MAX_ACTIONS as u32) {
        let Some(action_value) = actions.get_index(scope, index) else {
            continue;
        };
        let Some(action_object) = v8::Local::<v8::Object>::try_from(action_value).ok() else {
            throw_type_error(
                scope,
                "Failed to read a NotificationAction entry: the provided value is not an object.",
            );
            return None;
        };
        parsed.push(notification_action_from_options(scope, action_object)?);
    }
    Some(parsed)
}

fn notification_action_from_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    action: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerNotificationAction> {
    Some(ServiceWorkerNotificationAction {
        action: required_notification_action_string(scope, action, "action")?,
        title: required_notification_action_string(scope, action, "title")?,
        icon: optional_notification_action_string(scope, action, "icon")?,
        navigate: optional_notification_action_navigate(scope, action)?,
    })
}

fn required_notification_action_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    action: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<String> {
    let value = match webidl::property_result(
        scope,
        action,
        name,
        webidl::Context::member("NotificationAction", name),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => {
            throw_type_error(
                scope,
                &format!("Failed to read a NotificationAction entry: '{name}' is required."),
            );
            return None;
        }
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    match webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("NotificationAction", name),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn optional_notification_action_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    action: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<String> {
    let value = match webidl::property_result(
        scope,
        action,
        name,
        webidl::Context::member("NotificationAction", name),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(String::new()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    match webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("NotificationAction", name),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn optional_notification_action_navigate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    action: v8::Local<'s, v8::Object>,
) -> Option<Option<Url>> {
    let value = match webidl::property_result(
        scope,
        action,
        "navigate",
        webidl::Context::member("NotificationAction", "navigate"),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(None),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let value = match webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("NotificationAction", "navigate"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Some(base_url) = notification_base_url(scope) else {
        return Some(Url::parse(&value).ok());
    };
    Some(base_url.join(&value).ok())
}

fn notification_base_url(scope: &mut v8::PinScope<'_, '_>) -> Option<Url> {
    crate::worker::worker_current_script_url(scope).or_else(|| {
        context_host_ptr_from_global_bridge(scope)
            .map(|host_ptr| unsafe { (&*host_ptr).document_url().clone() })
    })
}

pub(crate) fn build_notification_object_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: &ServiceWorkerNotificationSnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = global_constructor_object(scope, "Notification")
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let title = v8_string(scope, &snapshot.title)?;
    let data = crate::context_bootstrap::structured_deserialize_value(scope, &snapshot.data)
        .unwrap_or_else(|| v8::null(scope).into());
    let init = ObjectLiteralDeclaration::bind(scope);
    init.set_string_property(scope, "data", data);
    if let Some(tag) = v8_string(scope, &snapshot.tag) {
        init.set_string_property(scope, "tag", tag.into());
    }
    apply_notification_metadata_init(scope, &init, &snapshot.metadata);
    let notification = constructor.new_instance(scope, &[title.into(), init.into_value()])?;
    set_notification_actions_slot(scope, notification, &snapshot.actions);
    set_notification_record_slots(scope, notification, snapshot.registration_id, snapshot.id);
    Some(notification)
}

fn initialize_notification_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    notification: v8::Local<'s, v8::Object>,
    title: String,
    tag: String,
    metadata: ServiceWorkerNotificationMetadata,
    data: v8::Local<'s, v8::Value>,
    actions: Vec<ServiceWorkerNotificationAction>,
    record: Option<(ServiceWorkerRegistrationId, u64)>,
) {
    let action_array: v8::Local<'_, v8::Value> = notification_actions_array(scope, &actions).into();
    let vibrate_array: v8::Local<'_, v8::Value> =
        notification_vibrate_array(scope, &metadata.vibrate).into();
    let silent = metadata
        .silent
        .map(|silent| v8::Boolean::new(scope, silent).into())
        .unwrap_or_else(|| v8::null(scope).into());
    NotificationObjectDeclaration {
        title,
        data,
        tag,
        dir: metadata.dir,
        lang: metadata.lang,
        body: metadata.body,
        icon: metadata.icon,
        image: metadata.image,
        badge: metadata.badge,
        vibrate: vibrate_array,
        timestamp: metadata.timestamp.unwrap_or(0) as f64,
        renotify: metadata.renotify,
        silent,
        require_interaction: metadata.require_interaction,
        actions: action_array,
        event_target_slot: (),
        ordered_handlers: (),
        onclick: (),
        onerror: (),
        onshow: (),
        onclose: (),
        add_event_listener: (),
        remove_event_listener: (),
        dispatch_event: (),
        close: (),
    }
    .initialize(scope, notification)
    .expect("Notification declaration should initialize");
    if let Some((registration_id, record_id)) = record {
        set_notification_record_slots(scope, notification, registration_id, record_id);
    }
}

fn apply_notification_metadata_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: &ObjectLiteralDeclaration<'s>,
    metadata: &ServiceWorkerNotificationMetadata,
) {
    set_string_property(scope, init, "dir", &metadata.dir);
    set_string_property(scope, init, "lang", &metadata.lang);
    set_string_property(scope, init, "body", &metadata.body);
    set_string_property(scope, init, "icon", &metadata.icon);
    set_string_property(scope, init, "image", &metadata.image);
    set_string_property(scope, init, "badge", &metadata.badge);
    let vibrate = notification_vibrate_array(scope, &metadata.vibrate);
    init.set_string_property(scope, "vibrate", vibrate.into());
    if let Some(timestamp) = metadata.timestamp {
        init.set_string_property(
            scope,
            "timestamp",
            v8::Number::new(scope, timestamp as f64).into(),
        );
    }
    init.set_string_property(
        scope,
        "renotify",
        v8::Boolean::new(scope, metadata.renotify).into(),
    );
    let silent = metadata
        .silent
        .map(|silent| v8::Boolean::new(scope, silent).into())
        .unwrap_or_else(|| v8::null(scope).into());
    init.set_string_property(scope, "silent", silent);
    init.set_string_property(
        scope,
        "requireInteraction",
        v8::Boolean::new(scope, metadata.require_interaction).into(),
    );
}

fn set_string_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: &ObjectLiteralDeclaration<'s>,
    name: &'static str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        object.set_string_property(scope, name, value.into());
    }
}

fn notification_vibrate_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    vibrate: &[u32],
) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, vibrate.len() as i32);
    for (index, value) in vibrate.iter().copied().enumerate() {
        let _ = array.set_index(
            scope,
            index as u32,
            v8::Integer::new_from_unsigned(scope, value).into(),
        );
    }
    array
}

fn set_notification_actions_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    notification: v8::Local<'s, v8::Object>,
    actions: &[ServiceWorkerNotificationAction],
) {
    let actions: v8::Local<'_, v8::Value> = notification_actions_array(scope, actions).into();
    set_private_value(scope, notification, NOTIFICATION_ACTIONS_SLOT, actions);
}

fn notification_actions_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    actions: &[ServiceWorkerNotificationAction],
) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, actions.len() as i32);
    for (index, action) in actions.iter().enumerate() {
        let object = ObjectLiteralDeclaration::bind(scope);
        if let Some(value) = v8_string(scope, &action.action) {
            object.set_string_property(scope, "action", value.into());
        }
        if let Some(value) = v8_string(scope, &action.title) {
            object.set_string_property(scope, "title", value.into());
        }
        if let Some(value) = v8_string(scope, &action.icon) {
            object.set_string_property(scope, "icon", value.into());
        }
        if let Some(navigate) = &action.navigate
            && let Some(value) = v8_string(scope, navigate.as_str())
        {
            object.set_string_property(scope, "navigate", value.into());
        }
        let _ = array.set_index(scope, index as u32, object.into_value());
    }
    array
}

fn set_notification_record_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    notification: v8::Local<'s, v8::Object>,
    registration_id: ServiceWorkerRegistrationId,
    record_id: u64,
) {
    set_private_value(
        scope,
        notification,
        NOTIFICATION_RECORD_ID_SLOT,
        v8::BigInt::new_from_u64(scope, record_id).into(),
    );
    set_private_value(
        scope,
        notification,
        NOTIFICATION_RECORD_REGISTRATION_ID_SLOT,
        v8::BigInt::new_from_u64(scope, registration_id.as_u64()).into(),
    );
}

fn notification_title_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), NOTIFICATION_TITLE_SLOT)
            .unwrap_or_else(|| v8str(scope, "").into()),
    );
}

fn notification_data_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), NOTIFICATION_DATA_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn notification_tag_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_string_slot_getter(scope, args, &mut rv, NOTIFICATION_TAG_SLOT, "");
}

fn notification_dir_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_string_slot_getter(scope, args, &mut rv, NOTIFICATION_DIR_SLOT, "auto");
}

fn notification_lang_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_string_slot_getter(scope, args, &mut rv, NOTIFICATION_LANG_SLOT, "");
}

fn notification_body_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_string_slot_getter(scope, args, &mut rv, NOTIFICATION_BODY_SLOT, "");
}

fn notification_icon_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_string_slot_getter(scope, args, &mut rv, NOTIFICATION_ICON_SLOT, "");
}

fn notification_image_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_string_slot_getter(scope, args, &mut rv, NOTIFICATION_IMAGE_SLOT, "");
}

fn notification_badge_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_string_slot_getter(scope, args, &mut rv, NOTIFICATION_BADGE_SLOT, "");
}

fn notification_vibrate_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), NOTIFICATION_VIBRATE_SLOT)
            .unwrap_or_else(|| v8::Array::new(scope, 0).into()),
    );
}

fn notification_timestamp_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), NOTIFICATION_TIMESTAMP_SLOT)
            .unwrap_or_else(|| v8::Number::new(scope, 0.0).into()),
    );
}

fn notification_renotify_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_bool_slot_getter(scope, args, &mut rv, NOTIFICATION_RENOTIFY_SLOT);
}

fn notification_silent_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), NOTIFICATION_SILENT_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn notification_require_interaction_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    notification_bool_slot_getter(scope, args, &mut rv, NOTIFICATION_REQUIRE_INTERACTION_SLOT);
}

fn notification_string_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    slot: &'static str,
    default_value: &'static str,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8str(scope, default_value).into()),
    );
}

fn notification_bool_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    slot: &'static str,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::Boolean::new(scope, false).into()),
    );
}

fn notification_actions_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), NOTIFICATION_ACTIONS_SLOT)
            .unwrap_or_else(|| v8::Array::new(scope, 0).into()),
    );
}

fn notification_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let this = args.this();
    if !notification_receiver_branded(scope, this) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(record_id) = notification_private_u64(scope, this, NOTIFICATION_RECORD_ID_SLOT) else {
        return;
    };
    let Some(registration_id) =
        notification_private_u64(scope, this, NOTIFICATION_RECORD_REGISTRATION_ID_SLOT)
    else {
        return;
    };
    let registration_id = ServiceWorkerRegistrationId::from_u64_for_binding(registration_id);
    if let Some((current_registration_id, version_id, parent_tx)) =
        crate::worker::service_worker_runtime_identity(scope)
    {
        if current_registration_id == registration_id {
            let _ = parent_tx.send(
                crate::worker::WorkerToParentMessage::ServiceWorkerCloseNotification(
                    crate::runtime::ServiceWorkerCloseNotification {
                        registration_id,
                        version_id,
                        notification_id: record_id,
                    },
                ),
            );
        }
        return;
    }
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &*host_ptr }.close_service_worker_notification(registration_id, record_id);
    }
}

fn notification_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::Notification::is_instance(scope, receiver)
}

fn notification_event_target_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_add_event_listener_callback(scope, args, rv);
}

fn notification_event_target_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_remove_event_listener_callback(scope, args, rv);
}

fn notification_event_target_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !notification_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_dispatch_event_callback(scope, args, rv);
}

fn notification_private_u64<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    notification: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<u64> {
    let value = get_private_value(scope, notification, slot)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, lossless) = value.u64_value();
    lossless.then_some(id)
}

fn notification_permission_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(notification_permission_value(scope));
}

fn notification_request_permission_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let value = notification_permission_value(scope);
    let _ = resolver.resolve(scope, value);
    rv.set(resolver.get_promise(scope).into());
}

fn notification_permission_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    let state = notification_permission_state(scope);
    if let Some(value) = v8_string(scope, &state) {
        return value.into();
    }
    v8str(scope, "default").into()
}

fn notification_permission_state(scope: &mut v8::PinScope<'_, '_>) -> String {
    if let Some(state) = crate::worker::worker_notification_permission_state(scope) {
        return state;
    }

    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return "default".to_owned();
    };
    let host = unsafe { &*host_ptr };
    if !moli_url::is_potentially_trustworthy_url(host.document_url()) {
        return "denied".to_owned();
    }
    normalize_notification_permission_state(host.permission_state("notifications")).to_owned()
}

fn normalize_notification_permission_state(state: &str) -> &'static str {
    match state {
        "granted" => "granted",
        "denied" => "denied",
        _ => "default",
    }
}
