//! Bounded OfflineAudioContext constructor overloads and render-size metadata.

use super::*;
use webidl::WebIdlConverter;

const RENDER_QUANTUM_SIZE: &str = "__moliAudioRenderQuantumSize";
const DEFAULT_QUANTUM: u32 = 128;

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "AudioContextRenderSizeCategory", rename_all = "lowercase")]
enum RenderSizeCategory {
    Default,
    Hardware,
}

#[derive(Clone, Copy)]
enum RenderSizeHint {
    Category(RenderSizeCategory),
    Frames(u32),
}

fn render_size_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<RenderSizeHint, webidl::WebIdlError> {
    let context = webidl::Context::member("OfflineAudioContextOptions", name);
    let Some(value) = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(RenderSizeHint::Category(RenderSizeCategory::Default));
    };
    // For (enum or unsigned long), only a Number selects the numeric branch.
    // Objects, including boxed Numbers, use the enum's DOMString conversion.
    if value.is_number() {
        Ok(RenderSizeHint::Frames(
            webidl::UnsignedLong::convert(scope, value, context, &())?.0,
        ))
    } else {
        Ok(RenderSizeHint::Category(
            webidl::EnumValue::<RenderSizeCategory>::convert(scope, value, context, &())?.0,
        ))
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "OfflineAudioContextOptions")]
struct Options {
    // Dictionary members are converted in WebIDL lexical order. This bounded
    // interface retains required length; the newer nullable/chunked rendering
    // overload needs a different backend and lifecycle.
    #[webidl(required)]
    length: u32,
    #[webidl(default = 1)]
    number_of_channels: u32,
    #[webidl(with = render_size_member)]
    render_size_hint: RenderSizeHint,
    #[webidl(required)]
    sample_rate: f32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OfflineAudioContext")]
struct PositionalArgs {
    #[webidl(required)]
    number_of_channels: u32,
    #[webidl(required)]
    length: u32,
    #[webidl(required)]
    sample_rate: f32,
}

fn options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<Options> {
    // The effective overload set has arities 1 and 3. Resolve it before
    // invoking any conversion hook; arguments beyond the longest are ignored.
    match args.length() {
        1 => {
            let result = webidl::parse_dictionary::<Options>(
                scope,
                args.get(0),
                webidl::Context::argument("OfflineAudioContext", 1),
            );
            match result {
                Ok(Some(options)) => Some(options),
                Ok(None) => {
                    throw_type_error(scope, "OfflineAudioContextOptions requires length.");
                    None
                }
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    None
                }
            }
        }
        3.. => webidl::parse_args::<PositionalArgs>(scope, args).map(|args| Options {
            length: args.length,
            number_of_channels: args.number_of_channels,
            render_size_hint: RenderSizeHint::Category(RenderSizeCategory::Default),
            sample_rate: args.sample_rate,
        }),
        _ => {
            throw_type_error(
                scope,
                "OfflineAudioContext requires one or three arguments.",
            );
            None
        }
    }
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "OfflineAudioContext requires new.");
        return;
    }
    let Some(options) = options(scope, &args) else {
        return;
    };
    if !format::validate(
        scope,
        options.number_of_channels,
        options.length,
        options.sample_rate,
    ) {
        return;
    }
    let quantum = match options.render_size_hint {
        RenderSizeHint::Category(RenderSizeCategory::Default | RenderSizeCategory::Hardware) => {
            DEFAULT_QUANTUM
        }
        RenderSizeHint::Frames(frames) => {
            let maximum = (6.0 * f64::from(options.sample_rate)).floor() as u32;
            if !(1..=maximum).contains(&frames) {
                throw_dom_exception(
                    scope,
                    "NotSupportedError",
                    9,
                    "Unsupported audio render quantum size.",
                );
                return;
            }
            frames
        }
    };

    let context = args.this();
    let destination = audio_destination_node(scope, context);
    let compressors = v8::Array::new(scope, 0);
    let modules = new_web_audio_map_object(scope);
    let module_list = v8::Array::new(scope, 0);
    let processors = new_web_audio_map_object(scope);
    set_private_value(scope, context, AUDIO_CONTEXT_MODULES_SLOT, modules.into());
    set_private_value(
        scope,
        context,
        AUDIO_CONTEXT_MODULE_LIST_SLOT,
        module_list.into(),
    );
    set_private_value(
        scope,
        context,
        AUDIO_CONTEXT_PROCESSORS_SLOT,
        processors.into(),
    );
    set_web_audio_number_slot(scope, context, RENDER_QUANTUM_SIZE, f64::from(quantum));
    let length = f64::from(options.length);
    let sample_rate = f64::from(options.sample_rate);
    OfflineAudioContextObjectDeclaration::new(
        0.0,
        length,
        sample_rate,
        length,
        sample_rate,
        f64::from(options.number_of_channels),
        compressors,
        OFFLINE_AUDIO_LISTENERS_SLOT,
        "suspended",
        destination,
    )
    .initialize(scope, context)
    .expect("OfflineAudioContext declaration should initialize object");
    rv.set(context.into());
}

pub(super) fn render_quantum_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let quantum = web_audio_number_slot(scope, args.this(), RENDER_QUANTUM_SIZE)
        .unwrap_or(f64::from(DEFAULT_QUANTUM));
    rv.set(v8::Number::new(scope, quantum).into());
}
