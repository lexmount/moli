use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node.appendChild")]
pub(in crate::native_bridge) struct AppendChildArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    pub node: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node.insertBefore")]
pub(in crate::native_bridge) struct InsertBeforeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    pub node: v8::Local<'s, v8::Object>,
    #[webidl(required, nullable, interface = web_api_interfaces::Node)]
    pub child: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node.replaceChild")]
pub(in crate::native_bridge) struct ReplaceChildArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    pub node: v8::Local<'s, v8::Object>,
    #[webidl(required, interface = web_api_interfaces::Node)]
    pub child: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node")]
pub(in crate::native_bridge) struct RequiredNodeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    pub node: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node")]
pub(in crate::native_bridge) struct NullableNodeArgs<'s> {
    #[webidl(required, nullable, interface = web_api_interfaces::Node)]
    pub node: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node")]
pub(in crate::native_bridge) struct NodeNamespaceArgs {
    #[webidl(required, nullable)]
    pub namespace: Option<String>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "GetRootNodeOptions")]
pub(in crate::native_bridge) struct GetRootNodeOptions {
    #[webidl(default = false)]
    pub composed: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node.getRootNode")]
pub(in crate::native_bridge) struct GetRootNodeArgs {
    #[webidl(dictionary)]
    pub options: GetRootNodeOptions,
}
