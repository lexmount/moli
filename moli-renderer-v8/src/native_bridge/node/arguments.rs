use super::*;

/// A WebIDL Node argument retains the original object, including registered
/// native proxies and Attr objects that do not live in the tree arena.
pub(in crate::native_bridge) struct NodeReference<'s>(pub v8::Local<'s, v8::Object>);

impl<'s> webidl::WebIdlConverter<'s> for NodeReference<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let object = v8::Local::<v8::Object>::try_from(value)
            .map_err(|_| webidl::WebIdlError::cannot_convert(context, "Node"))?;
        if !web_api_interfaces::Node::is_instance(scope, object) {
            return Err(webidl::WebIdlError::cannot_convert(context, "Node"));
        }
        Ok(Self(object))
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node")]
pub(in crate::native_bridge) struct RequiredNodeArgs<'s> {
    #[webidl(required, converter = "raw")]
    pub node: NodeReference<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Node")]
pub(in crate::native_bridge) struct NullableNodeArgs<'s> {
    #[webidl(required, nullable, converter = "raw")]
    pub node: Option<NodeReference<'s>>,
}
