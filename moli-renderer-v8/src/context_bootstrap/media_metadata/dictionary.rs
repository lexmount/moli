use crate::{util::new_null_prototype_object, webidl};

#[derive(Default)]
pub(super) struct Text(pub(super) Vec<u16>);

impl<'s> webidl::WebIdlConverter<'s> for Text {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::convert::<webidl::DomString16>(scope, value, context).map(|value| Self(value.0))
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaImage")]
pub(super) struct MediaImage {
    #[webidl(converter = "raw", default = Text::default())]
    pub(super) sizes: Text,
    #[webidl(required, converter = "usv_string")]
    pub(super) src: String,
    #[webidl(name = "type", converter = "raw", default = Text::default())]
    pub(super) image_type: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ChapterInformationInit")]
pub(super) struct ChapterInformationInit {
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) artwork: webidl::Sequence<MediaImage>,
    #[webidl(converter = "double", default = 0.0)]
    pub(super) start_time: f64,
    #[webidl(converter = "raw", default = Text::default())]
    pub(super) title: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaMetadataInit")]
pub(super) struct MediaMetadataInit {
    #[webidl(converter = "raw", default = Text::default())]
    pub(super) album: Text,
    #[webidl(converter = "raw", default = Text::default())]
    pub(super) artist: Text,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) artwork: webidl::Sequence<MediaImage>,
    #[webidl(converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) chapter_info: webidl::Sequence<ChapterInformationInit>,
    #[webidl(converter = "raw", default = Text::default())]
    pub(super) title: Text,
}

pub(super) fn convert_dictionary<'s, T: webidl::WebIdlDictionary<'s>>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<T, webidl::WebIdlError> {
    // An empty dictionary cannot inherit author-installed Object.prototype members.
    let object = webidl::dictionary_value(value, context)?
        .unwrap_or_else(|| new_null_prototype_object(scope));
    webidl::parse_dictionary_object(scope, object)
}

impl<'s> webidl::WebIdlConverter<'s> for MediaImage {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        convert_dictionary(scope, value, context)
    }
}

impl<'s> webidl::WebIdlConverter<'s> for ChapterInformationInit {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        convert_dictionary(scope, value, context)
    }
}
