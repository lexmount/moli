//! Network-facing WebIDL options. Request and cache owners use the converted values.
use moli_webidl as webidl;

#[derive(Debug, webidl::WebIdlDictionary)]
#[webidl(prefix = "CacheQueryOptions")]
pub struct CacheQueryOptions {
    #[webidl(default = false)]
    pub ignore_method: bool,
    #[webidl(default = false)]
    pub ignore_search: bool,
    #[webidl(default = false)]
    pub ignore_vary: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MultiCacheQueryOptions")]
pub struct MultiCacheQueryOptions {
    #[webidl(inherit)]
    pub base: CacheQueryOptions,
    #[webidl(converter = "dom_string16")]
    pub cache_name: Option<Vec<u16>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EventSource")]
pub struct EventSourceConstructorArgs<'s> {
    #[webidl(
        required,
        name = "url",
        converter = "usv_string",
        missing_message = "Failed to construct 'EventSource': 1 argument required, but only 0 present."
    )]
    pub url: String,
    #[webidl(index = 1, converter = "raw")]
    pub options: Option<v8::Local<'s, v8::Value>>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "EventSourceInit")]
pub struct EventSourceInit {
    #[webidl(default = false)]
    pub with_credentials: bool,
}

pub fn event_source_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Option<EventSourceInit> {
    let Some(value) = value else {
        return Some(EventSourceInit::default());
    };
    match webidl::parse_dictionary::<EventSourceInit>(
        scope,
        value,
        webidl::Context::argument("EventSource", 2),
    ) {
        Ok(options) => Some(options.unwrap_or_default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}
