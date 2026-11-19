use moli_webidl as webidl;

#[derive(Clone, Copy, Default, PartialEq, Eq, webidl::WebIdlDictionary)]
#[webidl(prefix = "ToolAnnotations", snapshot)]
pub struct ToolAnnotations {
    #[webidl(name = "consequentialHint", default = false)]
    #[webidl(snapshot_order = 1)]
    pub consequential: bool,
    #[webidl(default = false)]
    #[webidl(snapshot_order = 3)]
    pub debugging: bool,
    #[webidl(name = "readOnlyHint", default = false)]
    #[webidl(snapshot_order = 0)]
    pub read_only: bool,
    #[webidl(name = "untrustedContentHint", default = false)]
    #[webidl(snapshot_order = 2)]
    pub untrusted_content: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ModelContextTool")]
pub struct ToolDefinition<'s> {
    #[webidl(dictionary)]
    pub annotations: Option<ToolAnnotations>,
    #[webidl(required, converter = "dom_string")]
    pub description: String,
    #[webidl(required, converter = "callback_function")]
    pub execute: webidl::WebIdlCallbackFunction,
    #[webidl(name = "inputSchema")]
    pub input_schema: Option<v8::Local<'s, v8::Object>>,
    #[webidl(required, converter = "dom_string")]
    pub name: String,
    #[webidl(converter = "usv_string")]
    pub title: Option<String>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ModelContextGetToolOptions")]
pub struct GetOptions {
    #[webidl(name = "fromOrigins", converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub from_origins: webidl::Sequence<webidl::UsvString>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ModelContextRegisterToolOptions")]
pub struct RegisterOptions<'s> {
    #[webidl(name = "exposedTo", converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub exposed_to: webidl::Sequence<webidl::UsvString>,
    #[webidl(with = signal_member)]
    pub signal: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ExecuteToolOptions")]
pub struct ExecuteOptions<'s> {
    #[webidl(with = signal_member)]
    pub signal: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RegisteredTool", snapshot)]
pub struct ToolReference<'s> {
    #[webidl(dictionary)]
    #[webidl(snapshot_order = 6)]
    pub annotations: Option<ToolAnnotations>,
    #[webidl(required, converter = "dom_string")]
    #[webidl(snapshot_order = 1)]
    pub description: String,
    #[webidl(name = "inputSchema")]
    #[webidl(snapshot_order = 5)]
    pub input_schema: Option<v8::Local<'s, v8::Object>>,
    #[webidl(required, converter = "dom_string")]
    #[webidl(snapshot_order = 0)]
    pub name: String,
    #[webidl(required, converter = "usv_string")]
    #[webidl(snapshot_order = 3)]
    pub origin: String,
    #[webidl(converter = "dom_string")]
    #[webidl(snapshot_order = 2)]
    pub title: Option<String>,
    #[webidl(required, converter = "raw")]
    #[webidl(snapshot_order = 4)]
    pub window: v8::Local<'s, v8::Object>,
}

fn signal_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    webidl::optional_member(
        scope,
        object,
        name,
        webidl::Context::member("ModelContextTool", name),
    )
}
