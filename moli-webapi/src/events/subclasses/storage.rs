use super::*;
use moli_v8_util::v8_string;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

struct StorageReference<'scope>(v8::Local<'scope, v8::Object>);

impl webidl::WebIdlSnapshotValue for StorageReference<'_> {
    const SNAPSHOT_KIND: webidl::SnapshotKind = webidl::SnapshotKind::Identity;
}

impl<'s> moli_webapi_declare::WebApiValue<'s> for StorageReference<'s> {
    fn to_v8_value(&self, _: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        Some(self.0.into())
    }
}

impl<'scope> webidl::WebIdlConverter<'scope> for StorageReference<'scope> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'scope, '_>,
        value: v8::Local<'scope, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && crate::interfaces::Storage::is_instance(scope, object)
        {
            return Ok(Self(object));
        }
        Err(webidl::WebIdlError::cannot_convert(context, "Storage"))
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "StorageEventInit", snapshot)]
pub(super) struct StorageEventInitMembers<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, converter = "raw")]
    key: Option<webidl::DomString16>,
    #[webidl(name = "newValue", nullable, converter = "raw")]
    new_value: Option<webidl::DomString16>,
    #[webidl(name = "oldValue", nullable, converter = "raw")]
    old_value: Option<webidl::DomString16>,
    #[webidl(name = "storageArea", converter = "raw", nullable)]
    storage_area: Option<StorageReference<'s>>,
    #[webidl(default = "", converter = "usv_string")]
    url: String,
}

impl<'s> StorageEventInitMembers<'s> {
    fn from_native(
        key: Option<&[u16]>,
        old_value: Option<&[u16]>,
        new_value: Option<&[u16]>,
        url: &str,
        storage_area: Option<v8::Local<'s, v8::Value>>,
    ) -> Self {
        Self {
            base: EventInit::default(),
            key: key.map(|value| webidl::DomString16(value.to_vec())),
            old_value: old_value.map(|value| webidl::DomString16(value.to_vec())),
            new_value: new_value.map(|value| webidl::DomString16(value.to_vec())),
            url: url.to_owned(),
            storage_area: storage_area
                .and_then(|value| value.try_into().ok())
                .map(StorageReference),
        }
    }
}

impl StorageEventInitMembers<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
}

pub(super) fn parse_storage_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<StorageEventInitMembers<'s>> {
    super::super::init::parse_event_dictionary(scope, args, "StorageEvent")
}

pub(super) fn initialize_storage_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    parsed: StorageEventInitMembers<'s>,
) {
    parsed
        .initialize_snapshot(scope, event)
        .expect("StorageEvent state");
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "StorageEvent.initStorageEvent")]
struct InitStorageEventArgs<'s> {
    #[webidl(
        required,
        converter = "raw",
        missing_message = "Failed to execute 'initStorageEvent': 1 argument required."
    )]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, nullable, converter = "raw")]
    key: Option<webidl::DomString16>,
    #[webidl(name = "oldValue", index = 4, nullable, converter = "raw")]
    old_value: Option<webidl::DomString16>,
    #[webidl(name = "newValue", index = 5, nullable, converter = "raw")]
    new_value: Option<webidl::DomString16>,
    #[webidl(default = "", index = 6, converter = "usv_string")]
    url: String,
    #[webidl(index = 7, converter = "raw", nullable)]
    storage_area: Option<StorageReference<'s>>,
}

pub(super) fn storage_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = event_backing(scope, args.this());
    let Some(parsed) = webidl::parse_args::<InitStorageEventArgs>(scope, &args) else {
        return;
    };
    if !reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    ) {
        return;
    }
    define_storage_event_properties_utf16(
        scope,
        event,
        parsed.key.as_ref().map(|value| value.0.as_slice()),
        parsed.old_value.as_ref().map(|value| value.0.as_slice()),
        parsed.new_value.as_ref().map(|value| value.0.as_slice()),
        &parsed.url,
        parsed.storage_area.map(|value| value.0.into()),
    );
}

pub fn construct_original_storage_event_utf16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    key: Option<&[u16]>,
    old_value: Option<&[u16]>,
    new_value: Option<&[u16]>,
    url: &str,
    storage_area: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let event_type = v8_string(scope, event_type)?;
    let init = StorageEventInitMembers::from_native(key, old_value, new_value, url, storage_area);
    let wrapper = moli_v8_util::new_intrinsic_interface_instance(scope, "StorageEvent")?;
    super::construct_subclass(
        scope,
        wrapper,
        super::EventSubclassKind::StorageEvent,
        event_type,
        init.event_flags(),
        |scope, event| {
            initialize_storage_event(scope, event, init);
            true
        },
    )
}

fn define_storage_event_properties_utf16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    key: Option<&[u16]>,
    old_value: Option<&[u16]>,
    new_value: Option<&[u16]>,
    url: &str,
    storage_area: Option<v8::Local<'s, v8::Value>>,
) {
    let event = event_backing(scope, event);
    StorageEventInitMembers::from_native(key, old_value, new_value, url, storage_area)
        .initialize_own_snapshot(scope, event)
        .expect("StorageEvent native payload");
}
