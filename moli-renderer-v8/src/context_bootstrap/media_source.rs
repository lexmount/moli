//! MediaSource's initial closed state. Attachment, source buffers and decoding
//! require a media backend; this frontend does not synthesize an open source.

use super::{constructors::throw_dom_exception_value, media_queries};
use crate::{
    util::{get_private_object, get_private_value, set_private_value, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_web_mime::is_media_source_type_supported;
use moli_webapi_declare::WebApiFunctionTemplate;

pub(crate) mod source_buffer_list;
mod state;
pub(crate) use state::{MediaSourceObject, media_source_object};

const SOURCE_BUFFERS: &str = "__moliMediaSourceBuffers";
const ACTIVE_SOURCE_BUFFERS: &str = "__moliMediaSourceActiveBuffers";
const LISTENERS: &str = "__moliMediaSourceListeners";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSource.isTypeSupported")]
struct MediaSourceIsTypeSupportedArgs {
    #[webidl(required, name = "type")]
    media_type: String,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaSource)]
struct MediaSourceTemplateDeclaration {
    #[webapi(
        static_method = "isTypeSupported",
        enumerable,
        length = 1,
        callback = media_source_is_type_supported_callback
    )]
    is_type_supported: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaSource, receiver, enumerable)]
struct MediaSourcePrototype {
    #[webapi(accessor_property, getter = buffers, data = v8str(scope, SOURCE_BUFFERS))]
    source_buffers: (),
    #[webapi(accessor_property, getter = buffers, data = v8str(scope, ACTIVE_SOURCE_BUFFERS))]
    active_source_buffers: (),
    #[webapi(accessor_property, getter = ready_state)]
    ready_state: (),
    #[webapi(accessor_property, getter = duration, setter = set_duration)]
    duration: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "sourceopen"))]
    onsourceopen: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "sourceended"))]
    onsourceended: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "sourceclose"))]
    onsourceclose: (),
    #[webapi(method, length = 1, callback = add_source_buffer)]
    add_source_buffer: (),
    #[webapi(method, length = 1, callback = remove_source_buffer)]
    remove_source_buffer: (),
    #[webapi(method, length = 0, callback = end_of_stream)]
    end_of_stream: (),
    #[webapi(method, length = 2, callback = set_live_seekable_range)]
    set_live_seekable_range: (),
    #[webapi(method, length = 0, callback = clear_live_seekable_range)]
    clear_live_seekable_range: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSource.duration")]
struct DurationArgs {
    #[webidl(converter = "unrestricted_double", default = f64::NAN)]
    value: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSource.addSourceBuffer")]
struct AddArgs {
    #[webidl(required, name = "type")]
    media_type: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSource.removeSourceBuffer")]
struct RemoveArgs<'s> {
    #[webidl(required, name = "sourceBuffer", interface = web_api_interfaces::SourceBuffer)]
    _source_buffer: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "EndOfStreamError", rename_all = "lowercase")]
enum EndOfStreamError {
    Network,
    Decode,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSource.endOfStream")]
struct EndArgs {
    #[webidl(name = "error", converter = "enum")]
    _error: Option<EndOfStreamError>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSource.setLiveSeekableRange")]
struct RangeArgs {
    #[webidl(required, name = "start", converter = "double")]
    _start: f64,
    #[webidl(required, name = "end", converter = "double")]
    _end: f64,
}

pub(in crate::context_bootstrap) fn media_source_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "MediaSource constructor requires 'new'.");
        return;
    }
    state::initialize(scope, args.this());
    rv.set(args.this().into());
}

pub(in crate::context_bootstrap) fn install_media_source_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "MediaSource" {
        MediaSourceTemplateDeclaration::initialize_template(scope, template);
        MediaSourcePrototype::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    } else if interface_name == "SourceBufferList" {
        source_buffer_list::install(scope, template);
    }
}

fn media_source_is_type_supported_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<MediaSourceIsTypeSupportedArgs>(scope, &args) else {
        return;
    };
    rv.set(v8::Boolean::new(scope, is_media_source_type_supported(&parsed.media_type)).into());
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver).expect("MSE receiver was validated")
}

fn buffers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(list) = get_private_object(scope, source, &slot) {
        rv.set(list.into());
        return;
    }
    // A borrowed getter still returns a list from its owner's creation realm.
    let context = source
        .get_creation_context(scope)
        .expect("MediaSource has a creation realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let values = v8::Array::new(scope, 0);
    let list = source_buffer_list::build(scope, source, values);
    set_private_value(scope, source, &slot, list.into());
    rv.set(list.into());
}

fn ready_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8str(scope, "closed").into());
}

fn duration<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_double(f64::NAN);
}

fn closed_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception_value(scope, "The MediaSource is not open.", "InvalidStateError");
}

fn set_duration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DurationArgs>(scope, &args) else {
        return;
    };
    if parsed.value.is_nan() || parsed.value < 0.0 {
        throw_type_error(
            scope,
            "MediaSource duration must be nonnegative and not NaN.",
        );
        return;
    }
    closed_error(scope);
}

fn add_source_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AddArgs>(scope, &args) else {
        return;
    };
    if parsed.media_type.is_empty() {
        throw_type_error(scope, "Source buffer type must not be empty.");
    } else if !is_media_source_type_supported(&parsed.media_type) {
        throw_dom_exception_value(
            scope,
            "Unsupported source buffer type.",
            "NotSupportedError",
        );
    } else {
        closed_error(scope);
    }
}

fn remove_source_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if webidl::parse_args::<RemoveArgs>(scope, &args).is_some() {
        // Closed sources have no members, even if the argument is a genuine
        // SourceBuffer from another source.
        throw_dom_exception_value(
            scope,
            "Source buffer is not in this source.",
            "NotFoundError",
        );
    }
}

fn end_of_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if webidl::parse_args::<EndArgs>(scope, &args).is_some() {
        closed_error(scope);
    }
}

fn set_live_seekable_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if webidl::parse_args::<RangeArgs>(scope, &args).is_some() {
        // Finite double conversion precedes the algorithm; closed-state
        // rejection precedes its nonnegative/ordered range validation.
        closed_error(scope);
    }
}

fn clear_live_seekable_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    closed_error(scope);
}

fn handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let event = args.data().to_rust_string_lossy(scope);
    rv.set(
        get_private_value(scope, object, handler_slot(&event))
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let event = args.data().to_rust_string_lossy(scope);
    let slot = handler_slot(&event);
    let active = args.get(0).is_object();
    let handler = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, object, slot, handler);
    media_queries::simple_object_event_set_ordered_handler(
        scope, object, LISTENERS, &event, slot, active,
    );
}

fn handler_slot(event: &str) -> &'static str {
    match event {
        "sourceopen" => "__moliMediaSourceOnsourceopen",
        "sourceended" => "__moliMediaSourceOnsourceended",
        "sourceclose" => "__moliMediaSourceOnsourceclose",
        "addsourcebuffer" => "__moliSourceBufferListOnaddsourcebuffer",
        "removesourcebuffer" => "__moliSourceBufferListOnremovesourcebuffer",
        _ => unreachable!("MSE event handler callback data"),
    }
}
