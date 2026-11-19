use moli_v8_util::throw_type_error;
use moli_webidl as webidl;
use webidl::WebIdlConverter;

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "AudioContextRenderSizeCategory", rename_all = "lowercase")]
pub enum RenderSizeCategory {
    Default,
    Hardware,
}

#[derive(Clone, Copy)]
pub enum RenderSizeHint {
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
pub struct OfflineAudioContextOptions {
    // Dictionary members are converted in WebIDL lexical order. This bounded
    // interface retains required length; the newer nullable/chunked rendering
    // overload needs a different backend and lifecycle.
    #[webidl(required)]
    pub length: u32,
    #[webidl(default = 1)]
    pub number_of_channels: u32,
    #[webidl(with = render_size_member)]
    pub render_size_hint: RenderSizeHint,
    #[webidl(required)]
    pub sample_rate: f32,
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

pub fn parse_offline_audio_context_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<OfflineAudioContextOptions> {
    // The effective overload set has arities 1 and 3. Resolve it before
    // invoking any conversion hook; arguments beyond the longest are ignored.
    match args.length() {
        1 => {
            let result = webidl::parse_dictionary::<OfflineAudioContextOptions>(
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
        3.. => webidl::parse_args::<PositionalArgs>(scope, args).map(|args| {
            OfflineAudioContextOptions {
                length: args.length,
                number_of_channels: args.number_of_channels,
                render_size_hint: RenderSizeHint::Category(RenderSizeCategory::Default),
                sample_rate: args.sample_rate,
            }
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
