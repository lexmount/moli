use super::{
    SIMPLE_EVENT_TARGET_LISTENERS_SLOT, global_constructor_object,
    simple_event_target_handler_getter_callback, simple_event_target_handler_setter_callback,
    structured_clone_value_for_storage,
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
const NOTIFICATION_MAX_ACTIONS: usize = 2;

pub(crate) struct NotificationOptionsPayload {
    pub(crate) data: V8StructuredClonePayload,
    pub(crate) tag: String,
    pub(crate) metadata: ServiceWorkerNotificationMetadata,
    pub(crate) actions: Vec<ServiceWorkerNotificationAction>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Notification, receiver)]
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

    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    #[webapi(method, enumerable, callback = notification_close_callback)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Notification, receiver)]
struct NotificationPrototypeDeclaration {
    #[webapi(
        accessor_property = "onclick",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onclick"),
        setter_data = v8str(scope, "onclick"),
        enumerable
    )]
    onclick: (),
    #[webapi(
        accessor_property = "onerror",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onerror"),
        setter_data = v8str(scope, "onerror"),
        enumerable
    )]
    onerror: (),
    #[webapi(
        accessor_property = "onshow",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onshow"),
        setter_data = v8str(scope, "onshow"),
        enumerable
    )]
    onshow: (),
    #[webapi(
        accessor_property = "onclose",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onclose"),
        setter_data = v8str(scope, "onclose"),
        enumerable
    )]
    onclose: (),

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
#[webapi(plain)]
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
    let Some(parsed) = webidl::parse_args::<NotificationArgs>(scope, &args) else {
        return;
    };
    let Some(options) = notification_options(scope, parsed.options) else {
        return;
    };

    let notification = args.this();
    initialize_notification_object(
        scope,
        notification,
        parsed.title,
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

pub(in crate::context_bootstrap) fn finalize_notification_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Object>,
) -> anyhow::Result<()> {
    NotificationConstructorPermissionDeclaration::default()
        .initialize(scope, constructor)
        .map_err(|error| anyhow::anyhow!("failed to finalize Notification: {error}"))
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "NotificationDirection")]
enum NotificationDirection {
    #[webidl(token = "auto")]
    Auto,
    #[webidl(token = "ltr")]
    Ltr,
    #[webidl(token = "rtl")]
    Rtl,
}

struct VibrationPattern(Vec<u32>);

impl<'s> webidl::WebIdlConverter<'s> for VibrationPattern {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Some(values) = webidl::convert_optional_sequence::<webidl::UnsignedLong>(
            scope,
            value,
            context,
            &Default::default(),
        )? {
            return Ok(Self(values.0.into_iter().map(|value| value.0).collect()));
        }
        webidl::convert::<webidl::UnsignedLong>(scope, value, context)
            .map(|value| Self(vec![value.0]))
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NotificationAction")]
struct NotificationActionInit {
    #[webidl(required)]
    action: String,
    #[webidl(default = "", converter = "usv_string")]
    icon: String,
    #[webidl(converter = "usv_string")]
    navigate: Option<String>,
    #[webidl(required)]
    title: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Notification")]
struct NotificationArgs<'s> {
    #[webidl(required)]
    title: String,
    #[webidl(dictionary)]
    options: NotificationOptionsInit<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NotificationOptions")]
struct NotificationOptionsInit<'s> {
    #[webidl(sequence, converter = "raw", default = Vec::new())]
    actions: Vec<webidl::Dictionary<NotificationActionInit>>,
    #[webidl(default = "", converter = "usv_string")]
    badge: String,
    #[webidl(default = "")]
    body: String,
    #[webidl(converter = "raw")]
    data: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = NotificationDirection::Auto, converter = "enum")]
    dir: NotificationDirection,
    #[webidl(default = "", converter = "usv_string")]
    icon: String,
    #[webidl(default = "", converter = "usv_string")]
    image: String,
    #[webidl(default = "")]
    lang: String,
    #[webidl(default = false)]
    renotify: bool,
    #[webidl(default = false)]
    require_interaction: bool,
    #[webidl(nullable)]
    silent: Option<bool>,
    #[webidl(default = "")]
    tag: String,
    timestamp: Option<u64>,
    #[webidl(default = VibrationPattern(Vec::new()), converter = "raw")]
    vibrate: VibrationPattern,
}

impl NotificationOptionsInit<'_> {
    fn into_metadata(self) -> ServiceWorkerNotificationMetadata {
        ServiceWorkerNotificationMetadata {
            dir: match self.dir {
                NotificationDirection::Auto => "auto",
                NotificationDirection::Ltr => "ltr",
                NotificationDirection::Rtl => "rtl",
            }
            .to_owned(),
            lang: self.lang,
            body: self.body,
            icon: self.icon,
            image: self.image,
            badge: self.badge,
            vibrate: self.vibrate.0,
            timestamp: self.timestamp,
            renotify: self.renotify,
            silent: self.silent,
            require_interaction: self.require_interaction,
        }
    }
}

fn notification_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut options: NotificationOptionsInit<'s>,
) -> Option<NotificationObjectOptions<'s>> {
    if !options.actions.is_empty() {
        throw_type_error(
            scope,
            "Failed to construct 'Notification': non-persistent notifications do not support actions.",
        );
        return None;
    }
    let data = match options.data.take() {
        Some(data) => structured_clone_value_for_storage(scope, data)?,
        None => v8::null(scope).into(),
    };
    let tag = std::mem::take(&mut options.tag);
    Some(NotificationObjectOptions {
        data,
        tag,
        metadata: options.into_metadata(),
        actions: Vec::new(),
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
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(String, NotificationOptionsPayload)> {
    let parsed = webidl::parse_args::<NotificationArgs>(scope, args)?;
    let mut options = parsed.options;
    // The display limit is an algorithm step, after every sequence item and
    // its dictionary have been converted, including entries beyond the limit.
    let base_url = notification_base_url(scope);
    let actions = std::mem::take(&mut options.actions)
        .into_iter()
        .take(NOTIFICATION_MAX_ACTIONS)
        .map(|action| {
            let action = action.0;
            ServiceWorkerNotificationAction {
                action: action.action,
                title: action.title,
                icon: action.icon,
                navigate: action.navigate.and_then(|value| match &base_url {
                    Some(base) => base.join(&value).ok(),
                    None => Url::parse(&value).ok(),
                }),
            }
        })
        .collect();
    let data = options
        .data
        .take()
        .unwrap_or_else(|| v8::null(scope).into());
    let data = crate::context_bootstrap::structured_serialize_value_for_storage(scope, data)?;
    let tag = std::mem::take(&mut options.tag);
    Some((
        parsed.title,
        NotificationOptionsPayload {
            data,
            tag,
            metadata: options.into_metadata(),
            actions,
        },
    ))
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "GetNotificationOptions")]
struct GetNotificationOptions {
    #[webidl(default = "")]
    tag: String,
}

pub(crate) fn notification_get_options_tag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Option<String>> {
    match webidl::convert::<webidl::Dictionary<GetNotificationOptions>>(
        scope,
        value,
        webidl::Context::argument("ServiceWorkerRegistration.getNotifications", 1),
    ) {
        Ok(parsed) => Some((!parsed.0.tag.is_empty()).then_some(parsed.0.tag)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
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
        event_listeners: (),
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
