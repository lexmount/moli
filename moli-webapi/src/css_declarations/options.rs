use moli_webidl as webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleSheet")]
pub struct CssStyleSheetConstructorArgs<'s> {
    #[webidl(dictionary)]
    pub options: CssStyleSheetInit<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CSSStyleSheetInit")]
pub struct CssStyleSheetInit<'s> {
    // Constructed sheets have no title; the alternate flag has no selection
    // effect, but its WebIDL conversion still runs before the other members.
    #[webidl(name = "alternate", default = false)]
    _alternate: bool,
    #[webidl(name = "baseURL")]
    pub base_url: Option<String>,
    #[webidl(default = false)]
    pub disabled: bool,
    #[webidl(default = CssStyleSheetMedia::Text(String::new()), converter = "raw")]
    pub media: CssStyleSheetMedia<'s>,
}

pub enum CssStyleSheetMedia<'s> {
    Native(v8::Local<'s, v8::Object>),
    Text(String),
}

impl<'s> webidl::WebIdlConverter<'s> for CssStyleSheetMedia<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(list) = v8::Local::<v8::Object>::try_from(value)
            && crate::interfaces::MediaList::is_instance(scope, list)
        {
            return Ok(Self::Native(list));
        }
        webidl::convert::<webidl::DomString>(scope, value, context).map(|text| Self::Text(text.0))
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PropertyDefinition")]
pub struct PropertyDefinition {
    #[webidl(required)]
    pub name: String,
    #[webidl(default = "*")]
    pub syntax: String,
    #[webidl(required)]
    pub inherits: bool,
    pub initial_value: Option<String>,
}
