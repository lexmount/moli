//! ConvolverNode control state and native impulse-response acquisition.

use super::*;
use webidl::WebIdlDictionary;

const BUFFER: &str = "__moliConvolverBuffer";
const NORMALIZE: &str = "__moliConvolverNormalize";
const ACQUIRED_CONTENT: &str = "__moliConvolverAcquiredContent";
const ACQUIRED_NORMALIZE: &str = "__moliConvolverAcquiredNormalize";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ConvolverNode, enumerable, receiver)]
struct PrototypeDeclaration {
    #[webapi(accessor_property, getter = slot_getter, setter = buffer_setter, data = v8str(scope, BUFFER))]
    buffer: (),
    #[webapi(accessor_property, getter = slot_getter, setter = normalize_setter, data = v8str(scope, NORMALIZE))]
    normalize: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    PrototypeDeclaration::initialize_prototype_template(scope, template.prototype_template(scope));
}

#[derive(Default)]
struct Options<'s> {
    node: node::AudioNodeOptions,
    buffer: Option<v8::Local<'s, v8::Object>>,
    disable_normalization: bool,
}

fn parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Options<'s>, webidl::WebIdlError> {
    let mut options = Options::default();
    let Some(object) =
        webidl::dictionary_value(value, webidl::Context::argument("ConvolverNode", 2))?
    else {
        return Ok(options);
    };
    options.node = node::AudioNodeOptions::parse_dictionary(scope, object)?;
    let context = webidl::Context::member("ConvolverOptions", "buffer");
    if let Some(value) = webidl::property_result(scope, object, "buffer", context)? {
        options.buffer = buffer::nullable_value(scope, value, context)?;
    }
    options.disable_normalization = webidl::optional_member_or::<webidl::Boolean>(
        scope,
        object,
        "disableNormalization",
        webidl::Context::member("ConvolverOptions", "disableNormalization"),
        webidl::Boolean(false),
    )?
    .0;
    Ok(options)
}

fn set_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    value: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let content: v8::Local<v8::Value> = if let Some(buffer) = value {
        let context =
            graph::require_node_context(scope, node).expect("ConvolverNode should have a context");
        // Read native metadata, so overriding public getters cannot bypass the
        // channel/rate constraints or execute author code during configuration.
        if !matches!(buffer::channel_count(scope, buffer), 1 | 2 | 4)
            || buffer::sample_rate(scope, buffer) != audio_context_sample_rate(scope, context)
        {
            throw_dom_exception(
                scope,
                "NotSupportedError",
                9,
                "The impulse response needs 1, 2 or 4 channels and the context sample rate.",
            );
            return false;
        }
        buffer::acquire(scope, buffer).into()
    } else {
        v8::null(scope).into()
    };
    let normalize = get_private_value(scope, node, NORMALIZE)
        .expect("ConvolverNode should have normalization state");
    set_private_value(scope, node, ACQUIRED_NORMALIZE, normalize);
    set_private_value(scope, node, ACQUIRED_CONTENT, content);
    let value = value.map_or_else(|| v8::null(scope).into(), Into::into);
    set_private_value(scope, node, BUFFER, value);
    true
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    options: Options<'s>,
) -> bool {
    graph::initialize_node(scope, node, context);
    if !node::apply_options(scope, node, options.node) {
        return false;
    }
    graph::mark_unsupported_processor(scope, node);
    // Construction applies normalization before acquiring the response.
    set_private_value(
        scope,
        node,
        NORMALIZE,
        v8::Boolean::new(scope, !options.disable_normalization).into(),
    );
    set_buffer(scope, node, options.buffer)
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "ConvolverNode requires new.");
        return;
    }
    let context = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
    let Some(context) = context
        .filter(|context| web_api_interfaces::BaseAudioContext::is_instance(scope, *context))
    else {
        throw_type_error(scope, "ConvolverNode requires a BaseAudioContext.");
        return;
    };
    let options = match parse_options(scope, args.get(1)) {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    web_api_interfaces::initialize(scope, args.this(), "ConvolverNode")
        .expect("ConvolverNode brand should initialize");
    if initialize(scope, args.this(), context, options) {
        rv.set(args.this().into());
    }
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let realm = args
        .this()
        .get_creation_context(scope)
        .expect("Audio context should have a creation realm");
    let scope = &mut v8::ContextScope::new(scope, realm);
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "ConvolverNode",
    )
    .expect("ConvolverNode prototype should exist");
    let node = v8::Object::new(scope);
    let _ = node.set_prototype(scope, prototype.into());
    web_api_interfaces::initialize(scope, node, "ConvolverNode")
        .expect("ConvolverNode brand should initialize");
    if initialize(scope, node, args.this(), Options::default()) {
        rv.set(node.into());
    }
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ConvolverNode.buffer")]
struct BufferArgs<'s> {
    #[webidl(required, with = buffer_value)]
    value: Option<v8::Local<'s, v8::Object>>,
}

fn buffer_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    buffer::nullable_value(
        scope,
        args.get(index),
        webidl::Context::argument("ConvolverNode.buffer", 1),
    )
}

fn buffer_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<BufferArgs>(scope, &args) else {
        return;
    };
    set_buffer(scope, args.this(), parsed.value);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ConvolverNode.normalize")]
struct NormalizeArgs {
    #[webidl(required)]
    value: bool,
}

fn normalize_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<NormalizeArgs>(scope, &args) else {
        return;
    };
    set_private_value(
        scope,
        args.this(),
        NORMALIZE,
        v8::Boolean::new(scope, parsed.value).into(),
    );
}
