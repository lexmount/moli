use super::*;
use webidl::{Context, WebIdlConverter, WebIdlDictionarySerialize, WebIdlError};

#[derive(Clone, Default, webidl::WebIdlDictionary)]
#[webidl(snapshot)]
pub(super) struct Init {
    #[webidl(name = "baseURL", converter = "usv_string")]
    base_url: Option<String>,
    #[webidl(converter = "usv_string")]
    hash: Option<String>,
    #[webidl(converter = "usv_string")]
    hostname: Option<String>,
    #[webidl(converter = "usv_string")]
    password: Option<String>,
    #[webidl(converter = "usv_string")]
    pathname: Option<String>,
    #[webidl(converter = "usv_string")]
    port: Option<String>,
    #[webidl(converter = "usv_string")]
    protocol: Option<String>,
    #[webidl(converter = "usv_string")]
    search: Option<String>,
    #[webidl(converter = "usv_string")]
    username: Option<String>,
}

#[derive(Default, webidl::WebIdlDictionary)]
pub(super) struct Options {
    #[webidl(default = false)]
    pub(super) ignore_case: bool,
}

pub(super) enum Input {
    String(String),
    Init(Init),
}

impl<'s> WebIdlConverter<'s> for Input {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        if value.is_object() || value.is_null_or_undefined() {
            webidl::parse_dictionary(scope, value, context)
                .map(|value| Self::Init(value.unwrap_or_default()))
        } else {
            webidl::convert::<webidl::UsvString>(scope, value, context)
                .map(|value| Self::String(value.0))
        }
    }
}

impl Input {
    pub(super) fn library_input(&self) -> urlpattern::quirks::StringOrInit<'_> {
        use urlpattern::quirks::{StringOrInit, UrlPatternInit};
        match self {
            Self::String(value) => StringOrInit::String(value.as_str().into()),
            Self::Init(init) => StringOrInit::Init(UrlPatternInit {
                base_url: init.base_url.clone(),
                protocol: init.protocol.clone(),
                username: init.username.clone(),
                password: init.password.clone(),
                hostname: init.hostname.clone(),
                port: init.port.clone(),
                pathname: init.pathname.clone(),
                search: init.search.clone(),
                hash: init.hash.clone(),
            }),
        }
    }

    pub(super) fn snapshot<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::String(value) => v8_string(scope, value).map(Into::into),
            Self::Init(value) => value
                .to_js_snapshot(scope, webidl::SnapshotPrototype::Ordinary)
                .map(Into::into),
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URLPattern")]
pub(super) struct ConstructorArgs {
    #[webidl(with = input_arg)]
    pub(super) input: Input,
    #[webidl(with = options_arg)]
    pub(super) options: (Option<String>, Options),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URLPattern")]
pub(super) struct MatchArgs {
    #[webidl(with = input_arg)]
    pub(super) input: Input,
    #[webidl(converter = "usv_string")]
    pub(super) base_url: Option<String>,
}

fn input_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Input, WebIdlError> {
    webidl::convert::<Input>(scope, args.get(index), Context::argument("URLPattern", 1))
}

fn options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<(Option<String>, Options), WebIdlError> {
    let second = args.get(index);
    // With three arguments the first constructor overload applies. At two
    // arguments, objects/null/undefined select the options dictionary overload.
    if args.length() >= 3 || !(second.is_object() || second.is_null_or_undefined()) {
        let base = webidl::convert::<webidl::UsvString>(
            scope,
            second,
            Context::argument("URLPattern", 2),
        )?
        .0;
        let options =
            webidl::parse_dictionary(scope, args.get(2), Context::argument("URLPattern", 3))?
                .unwrap_or_default();
        Ok((Some(base), options))
    } else {
        let options = webidl::parse_dictionary(scope, second, Context::argument("URLPattern", 2))?
            .unwrap_or_default();
        Ok((None, options))
    }
}
