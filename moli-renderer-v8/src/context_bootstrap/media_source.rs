use crate::web_api_interfaces;
use crate::webidl;
use moli_web_mime::is_media_source_type_supported;
use moli_webapi_declare::WebApiFunctionTemplate;

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

pub(in crate::context_bootstrap) fn media_source_constructor_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn install_media_source_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "MediaSource" {
        MediaSourceTemplateDeclaration::initialize_template(scope, template);
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
