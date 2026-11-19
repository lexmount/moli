use moli_webidl as webidl;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "CheckVisibilityOptions")]
pub struct CheckVisibilityOptions {
    #[webidl(name = "checkOpacity", default = false)]
    pub check_opacity: bool,
    #[webidl(name = "checkVisibilityCSS", default = false)]
    pub check_visibility_css: bool,
    #[webidl(name = "contentVisibilityAuto", default = false)]
    pub content_visibility_auto: bool,
    #[webidl(name = "opacityProperty", default = false)]
    pub opacity_property: bool,
    #[webidl(name = "visibilityProperty", default = false)]
    pub visibility_property: bool,
}

pub fn parse_check_visibility_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<CheckVisibilityOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("Element.checkVisibility", 1);
    webidl::dictionary_arg(args, 0, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}
