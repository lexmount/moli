use super::*;

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
