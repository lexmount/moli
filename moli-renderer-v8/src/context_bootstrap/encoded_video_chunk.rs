//! Immutable encoded bytes; no encoder or decoder backend is needed.

use crate::{
    blob::array_buffer_from_bytes,
    util::{get_private_value, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const TYPE_SLOT: &str = "__moliEncodedVideoChunkType";
const TIMESTAMP_SLOT: &str = "__moliEncodedVideoChunkTimestamp";
const DURATION_SLOT: &str = "__moliEncodedVideoChunkDuration";
const DATA_SLOT: &str = "__moliEncodedVideoChunkData";

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "EncodedVideoChunkType")]
pub(crate) enum EncodedVideoChunkType {
    Key,
    Delta,
}

impl EncodedVideoChunkType {
    fn label(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Delta => "delta",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "EncodedVideoChunkInit")]
struct EncodedVideoChunkInit<'s> {
    #[webidl(required, converter = "raw")]
    data: webidl::AllowSharedBufferSource<'s>,
    #[webidl(converter = "enforce_range_unsigned_long_long")]
    duration: Option<u64>,
    #[webidl(required, converter = "enforce_range_long_long")]
    timestamp: i64,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    transfer: webidl::Sequence<v8::Local<'s, v8::ArrayBuffer>>,
    #[webidl(name = "type", required, converter = "enum")]
    kind: EncodedVideoChunkType,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EncodedVideoChunk")]
struct EncodedVideoChunkConstructorArgs<'s> {
    #[webidl(with = parse_init)]
    init: EncodedVideoChunkInit<'s>,
}

fn parse_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<EncodedVideoChunkInit<'s>, webidl::WebIdlError> {
    let context = webidl::Context::argument("EncodedVideoChunk", (index + 1) as usize);
    if args.length() <= index {
        return Err(webidl::WebIdlError::cannot_convert(
            context,
            "EncodedVideoChunkInit",
        ));
    }
    // Nullish dictionaries have no members, regardless of Object.prototype.
    // This dictionary requires data, so an empty dictionary cannot be valid.
    let object = webidl::dictionary_arg(args, index, context)?
        .ok_or_else(|| webidl::WebIdlError::cannot_convert(context, "EncodedVideoChunkInit"))?;
    webidl::parse_dictionary_object(scope, object)
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EncodedVideoChunk.copyTo")]
struct CopyToArgs<'s> {
    #[webidl(required, converter = "raw")]
    destination: webidl::AllowSharedBufferSource<'s>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::EncodedVideoChunk, require_prototype)]
struct EncodedVideoChunkObjectDeclaration<'s> {
    #[webapi(slot = TYPE_SLOT)]
    kind: &'static str,
    #[webapi(slot = TIMESTAMP_SLOT)]
    timestamp: f64,
    #[webapi(slot = DURATION_SLOT)]
    duration: v8::Local<'s, v8::Value>,
    // This buffer is private and never returned to author code.
    #[webapi(slot = DATA_SLOT)]
    bytes: v8::Local<'s, v8::ArrayBuffer>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::EncodedVideoChunk, enumerable, receiver)]
struct EncodedVideoChunkPrototypeDeclaration {
    #[webapi(accessor_property = "type", getter = slot_getter, data = v8str(scope, TYPE_SLOT))]
    kind: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, TIMESTAMP_SLOT))]
    timestamp: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, DURATION_SLOT))]
    duration: (),
    #[webapi(accessor_property, getter = byte_length_getter)]
    byte_length: (),
    #[webapi(method, length = 1, callback = copy_to_callback)]
    copy_to: (),
}

pub(in crate::context_bootstrap) fn install_encoded_video_chunk_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    EncodedVideoChunkPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn encoded_video_chunk_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "EncodedVideoChunk requires the 'new' operator.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<EncodedVideoChunkConstructorArgs>(scope, &args) else {
        return;
    };
    let init = parsed.init;
    for (index, buffer) in init.transfer.0.iter().enumerate() {
        if init.transfer.0[..index]
            .iter()
            .any(|other| buffer.strict_equals((*other).into()))
        {
            webidl::throw_dom_exception(
                scope,
                "DataCloneError",
                "Duplicate ArrayBuffer in transfer.",
            );
            return;
        }
    }
    for buffer in &init.transfer.0 {
        if buffer.was_detached() || !buffer.is_detachable() {
            webidl::throw_dom_exception(
                scope,
                "DataCloneError",
                "ArrayBuffer cannot be transferred.",
            );
            return;
        }
    }
    // Read only after all dictionary getters and transfer iteration have run.
    let payload = EncodedVideoChunkClonePayload {
        kind: init.kind,
        timestamp: init.timestamp,
        duration: init.duration,
        bytes: init.data.to_vec(scope),
    };
    let declaration = chunk_declaration(scope, payload);
    declaration
        .initialize(scope, args.this())
        .expect("EncodedVideoChunk slots initialize");
    for buffer in &init.transfer.0 {
        if buffer.detach(None) != Some(true) {
            webidl::throw_dom_exception(
                scope,
                "DataCloneError",
                "ArrayBuffer could not be transferred.",
            );
            return;
        }
    }
    rv.set(args.this().into());
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated EncodedVideoChunk receiver");
    rv.set(get_private_value(scope, target, &slot).expect("EncodedVideoChunk slot"));
}

fn chunk_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::ArrayBuffer> {
    let object = moli_webapi_declare::web_api_object_target(scope, object)
        .expect("validated EncodedVideoChunk receiver");
    v8::Local::try_from(
        get_private_value(scope, object, DATA_SLOT).expect("EncodedVideoChunk bytes"),
    )
    .expect("EncodedVideoChunk bytes are a private ArrayBuffer")
}

fn byte_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(chunk_bytes(scope, args.this()).byte_length() as u32);
}

fn copy_to_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CopyToArgs>(scope, &args) else {
        return;
    };
    let bytes = chunk_bytes(scope, args.this());
    if bytes.byte_length() > parsed.destination.byte_length() {
        throw_type_error(
            scope,
            "The destination is too small for this EncodedVideoChunk.",
        );
        return;
    }
    let backing = bytes.get_backing_store();
    let bytes = if backing.byte_length() == 0 {
        &[]
    } else {
        // SAFETY: private fixed storage is retained by `backing`, never exposed
        // or mutated, and no JavaScript runs during this copy.
        unsafe {
            std::slice::from_raw_parts(
                backing
                    .data()
                    .expect("nonempty chunk backing")
                    .as_ptr()
                    .cast::<u8>(),
                backing.byte_length(),
            )
        }
    };
    assert!(parsed.destination.write_bytes(scope, bytes));
    rv.set_undefined();
}

#[derive(Clone, Debug)]
pub(crate) struct EncodedVideoChunkClonePayload {
    pub(crate) kind: EncodedVideoChunkType,
    pub(crate) timestamp: i64,
    pub(crate) duration: Option<u64>,
    pub(crate) bytes: Vec<u8>,
}

fn chunk_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: EncodedVideoChunkClonePayload,
) -> EncodedVideoChunkObjectDeclaration<'s> {
    let duration = payload
        .duration
        .map(|duration| v8::Number::new(scope, duration as f64).into())
        .unwrap_or_else(|| v8::null(scope).into());
    let bytes =
        array_buffer_from_bytes(scope, payload.bytes).expect("EncodedVideoChunk bytes allocate");
    EncodedVideoChunkObjectDeclaration::new(
        payload.kind.label(),
        payload.timestamp as f64,
        duration,
        bytes,
    )
}

pub(crate) fn encoded_video_chunk_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<EncodedVideoChunkClonePayload> {
    if !web_api_interfaces::EncodedVideoChunk::is_instance(scope, object) {
        return None;
    }
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    let kind = match get_private_value(scope, object, TYPE_SLOT)?
        .to_rust_string_lossy(scope)
        .as_str()
    {
        "key" => EncodedVideoChunkType::Key,
        "delta" => EncodedVideoChunkType::Delta,
        _ => return None,
    };
    let timestamp = get_private_value(scope, object, TIMESTAMP_SLOT)?.number_value(scope)? as i64;
    let duration = get_private_value(scope, object, DURATION_SLOT)?;
    let duration = if duration.is_null() {
        None
    } else {
        Some(duration.number_value(scope)? as u64)
    };
    let backing = chunk_bytes(scope, object).get_backing_store();
    Some(EncodedVideoChunkClonePayload {
        kind,
        timestamp,
        duration,
        bytes: backing.iter().map(|byte| byte.get()).collect(),
    })
}

pub(crate) fn build_encoded_video_chunk_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: EncodedVideoChunkClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    if !super::exposed_interfaces::is_realm_interface_exposed(scope, "EncodedVideoChunk") {
        return None;
    }
    // Native allocation also works inside V8's no-JavaScript deserializer scope.
    chunk_declaration(scope, payload).bind(scope).ok()
}
