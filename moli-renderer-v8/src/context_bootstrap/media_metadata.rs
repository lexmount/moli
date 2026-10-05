//! Media metadata values and immutable chapter/artwork snapshots.
//!
//! The MediaSession association is retained independently of author properties.
//! Platform playback controls and artwork fetching require a media UI backend.

use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
};

use crate::{
    util::{
        context_host_ptr_from_context_slot, get_private_value, set_private_value, throw_type_error,
        v8_string, v8str,
    },
    web_api_interfaces, webidl,
};

mod dictionary;
use dictionary::{ChapterInformationInit, MediaImage, MediaMetadataInit, Text, convert_dictionary};

const TITLE: &str = "__moliMediaMetadataTitle";
const ARTIST: &str = "__moliMediaMetadataArtist";
const ALBUM: &str = "__moliMediaMetadataAlbum";
const ARTWORK: &str = "__moliMediaMetadataArtwork";
const CONVERTED_ARTWORK: &str = "__moliMediaMetadataConvertedArtwork";
const CHAPTERS: &str = "__moliMediaMetadataChapters";
const START_TIME: &str = "__moliChapterStartTime";
const SESSION: &str = "__moliMediaMetadataSession";
const METADATA: &str = "__moliMediaSessionMetadata";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaMetadata, require_prototype)]
struct MetadataObject<'s> {
    #[webapi(slot = TITLE)]
    title: v8::Local<'s, v8::String>,
    #[webapi(slot = ARTIST)]
    artist: v8::Local<'s, v8::String>,
    #[webapi(slot = ALBUM)]
    album: v8::Local<'s, v8::String>,
    #[webapi(slot = ARTWORK)]
    artwork: v8::Local<'s, v8::Array>,
    #[webapi(slot = CHAPTERS)]
    chapters: v8::Local<'s, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ChapterInformation, require_prototype)]
struct ChapterObject<'s> {
    #[webapi(slot = TITLE)]
    title: v8::Local<'s, v8::String>,
    #[webapi(slot = START_TIME)]
    start_time: f64,
    #[webapi(slot = ARTWORK)]
    artwork: v8::Local<'s, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ImageDictionary<'s> {
    sizes: v8::Local<'s, v8::String>,
    src: v8::Local<'s, v8::String>,
    #[webapi(data_property = "type")]
    image_type: v8::Local<'s, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaSession, require_prototype)]
struct SessionObject<'s> {
    #[webapi(slot = METADATA)]
    metadata: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaMetadata, enumerable, receiver)]
struct MetadataPrototype {
    #[webapi(accessor_property, getter = slot_getter, setter = text_setter, data = v8str(scope, TITLE))]
    title: (),
    #[webapi(accessor_property, getter = slot_getter, setter = text_setter, data = v8str(scope, ARTIST))]
    artist: (),
    #[webapi(accessor_property, getter = slot_getter, setter = text_setter, data = v8str(scope, ALBUM))]
    album: (),
    #[webapi(accessor_property, getter = artwork_getter, setter = artwork_setter)]
    artwork: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, CHAPTERS))]
    chapter_info: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ChapterInformation, enumerable, receiver)]
struct ChapterPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, TITLE))]
    title: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, START_TIME))]
    start_time: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, ARTWORK))]
    artwork: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaSession, enumerable, receiver)]
struct SessionPrototype {
    #[webapi(accessor_property, getter = slot_getter, setter = metadata_setter, data = v8str(scope, METADATA))]
    metadata: (),
}

pub(in crate::context_bootstrap) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface {
        "MediaMetadata" => MetadataPrototype::initialize_prototype_template(scope, prototype),
        "ChapterInformation" => ChapterPrototype::initialize_prototype_template(scope, prototype),
        "MediaSession" => SessionPrototype::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn build_session<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> anyhow::Result<v8::Local<'s, v8::Object>> {
    SessionObject::new(v8::null(scope).into())
        .bind(scope)
        .map_err(Into::into)
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "MediaMetadata requires the 'new' operator.");
        return;
    }
    let init = match convert_dictionary::<MediaMetadataInit>(
        scope,
        args.get(0),
        webidl::Context::argument("MediaMetadata", 1),
    ) {
        Ok(init) => init,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "MediaMetadata") {
        return;
    }
    let Some(artwork) = convert_artwork(scope, init.artwork.0) else {
        return;
    };
    let Some(chapters) = create_chapters(scope, init.chapter_info.0) else {
        return;
    };
    MetadataObject::new(
        text_value(scope, &init.title),
        text_value(scope, &init.artist),
        text_value(scope, &init.album),
        artwork,
        chapters,
    )
    .initialize(scope, args.this())
    .expect("MediaMetadata state initializes");
    rv.set(args.this().into());
}

fn text_value<'s>(scope: &mut v8::PinScope<'s, '_>, value: &Text) -> v8::Local<'s, v8::String> {
    v8::String::new_from_two_byte(scope, &value.0, v8::NewStringType::Normal)
        .expect("metadata DOMString")
}

fn convert_artwork<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    images: Vec<MediaImage>,
) -> Option<v8::Local<'s, v8::Array>> {
    let base = artwork_base_url(scope)?;
    let mut values = Vec::with_capacity(images.len());
    for image in images {
        let src = match base.join(&image.src) {
            Ok(src) => src,
            Err(_) => {
                throw_type_error(scope, "MediaImage.src is not a valid URL.");
                return None;
            }
        };
        let value = ImageDictionary::new(
            text_value(scope, &image.sizes),
            v8_string(scope, src.as_str()).expect("serialized artwork URL"),
            text_value(scope, &image.image_type),
        )
        .bind(scope)
        .expect("native artwork record");
        values.push(value.into());
    }
    Some(v8::Array::new_with_elements(scope, &values))
}

fn artwork_base_url(scope: &mut v8::PinScope<'_, '_>) -> Option<url::Url> {
    let current = scope.get_current_context();
    let host = unsafe { &*context_host_ptr_from_context_slot(current)? };
    // The entered context can still be the outer CDP script while a foreign
    // JavaScript function is executing. V8's incumbent context identifies that
    // calling script, independently of the native constructor/accessor realm.
    let caller = scope.get_incumbent_context().unwrap_or(current);
    let owner = host
        .window_execution_context_identity_for_access_check(caller)
        .map(|identity| identity.dispatch_scope())
        .unwrap_or_else(|| host.entered_owner_dispatch_scope(scope));
    let base = match owner {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_base_url(handle)
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup) => {
            host.lightweight_popup_request_base_url(scope, popup)
        }
        crate::native_bridge::OwnerDispatchScope::Top => None,
    };
    Some(base.unwrap_or_else(|| {
        host.dom_host()
            .document_base_url()
            .unwrap_or_else(|| host.document_url().clone())
    }))
}

fn freeze_artwork<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    images: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Array>> {
    let mut values = Vec::with_capacity(images.length() as usize);
    for index in 0..images.length() {
        let record = v8::Local::<v8::Object>::try_from(images.get_index(scope, index)?).ok()?;
        let sizes = record.get(scope, v8str(scope, "sizes").into())?;
        let src = record.get(scope, v8str(scope, "src").into())?;
        let image_type = record.get(scope, v8str(scope, "type").into())?;
        let image = ImageDictionary::new(
            sizes.try_into().ok()?,
            src.try_into().ok()?,
            image_type.try_into().ok()?,
        )
        .bind(scope)
        .expect("artwork dictionary in getter realm");
        if image.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
            return None;
        }
        values.push(image.into());
    }
    frozen_array(scope, &values)
}

fn frozen_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Array>> {
    let array = v8::Array::new_with_elements(scope, values);
    (array.set_integrity_level(scope, v8::IntegrityLevel::Frozen) == Some(true)).then_some(array)
}

fn create_chapters<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    chapters: Vec<ChapterInformationInit>,
) -> Option<v8::Local<'s, v8::Array>> {
    let mut values = Vec::with_capacity(chapters.len());
    for chapter in chapters {
        if chapter.start_time < 0.0 {
            throw_type_error(scope, "ChapterInformation.startTime must not be negative.");
            return None;
        }
        let artwork = convert_artwork(scope, chapter.artwork.0)?;
        let artwork = freeze_artwork(scope, artwork)?;
        let value = ChapterObject::new(
            text_value(scope, &chapter.title),
            chapter.start_time,
            artwork,
        )
        .bind(scope)
        .expect("ChapterInformation state binds");
        if value.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
            return None;
        }
        values.push(value.into());
    }
    frozen_array(scope, &values)
}

fn native_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver).expect("validated media receiver")
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = native_target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, target, &slot).expect("native media value"));
}

fn text_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::argument("MediaMetadata", 1),
    ) {
        Ok(value) => value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let slot = args.data().to_rust_string_lossy(scope);
    let target = native_target(scope, args.this());
    let value = text_value(scope, &Text(value.0));
    set_private_value(scope, target, &slot, value.into());
}

fn artwork_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = native_target(scope, args.this());
    if let Some(value) =
        get_private_value(scope, target, CONVERTED_ARTWORK).filter(|value| !value.is_undefined())
    {
        rv.set(value);
        return;
    }
    let images = get_private_value(scope, target, ARTWORK).expect("native artwork images");
    let Some(value) = freeze_artwork(scope, images.try_into().expect("artwork array")) else {
        return;
    };
    set_private_value(scope, target, CONVERTED_ARTWORK, value.into());
    rv.set(value.into());
}

fn artwork_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = webidl::Context::member("MediaMetadata", "artwork");
    // The attribute is FrozenArray<object>. Finish that boundary conversion
    // before its setter algorithm converts the records to MediaImage dictionaries.
    let objects = match webidl::convert::<webidl::Sequence<v8::Local<'s, v8::Object>>>(
        scope,
        args.get(0),
        context,
    ) {
        Ok(objects) => objects,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let mut images = Vec::with_capacity(objects.0.len());
    for object in objects.0 {
        match webidl::parse_dictionary_object(scope, object) {
            Ok(image) => images.push(image),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        }
    }
    let Some(images) = convert_artwork(scope, images) else {
        return;
    };
    let target = native_target(scope, args.this());
    set_private_value(scope, target, ARTWORK, images.into());
    set_private_value(
        scope,
        target,
        CONVERTED_ARTWORK,
        v8::undefined(scope).into(),
    );
}

fn metadata_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = args.get(0);
    let value = if value.is_null_or_undefined() {
        v8::null(scope).into()
    } else if v8::Local::<v8::Object>::try_from(value)
        .is_ok_and(|object| web_api_interfaces::MediaMetadata::is_instance(scope, object))
    {
        value
    } else {
        throw_type_error(
            scope,
            "MediaSession.metadata requires a MediaMetadata object or null.",
        );
        return;
    };
    let target = native_target(scope, args.this());
    let previous = get_private_value(scope, target, METADATA).expect("MediaSession metadata");
    if let Ok(previous) = v8::Local::<v8::Object>::try_from(previous) {
        let previous = native_target(scope, previous);
        set_private_value(scope, previous, SESSION, v8::null(scope).into());
    }
    set_private_value(scope, target, METADATA, value);
    if let Ok(metadata) = v8::Local::<v8::Object>::try_from(value) {
        let metadata = native_target(scope, metadata);
        set_private_value(scope, metadata, SESSION, args.this().into());
    }
}
