use crate::context_bootstrap::is_window_receiver;
use crate::web_api_interfaces;
use crate::webidl;

/// Window-valued event arguments and dictionary members share native identity.
pub(in crate::context_bootstrap) struct WindowReference<'s>(v8::Local<'s, v8::Object>);

impl<'s> WindowReference<'s> {
    pub(in crate::context_bootstrap) fn into_value(self) -> v8::Local<'s, v8::Value> {
        self.0.into()
    }
}

impl<'s> webidl::WebIdlConverter<'s> for WindowReference<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(window) = v8::Local::<v8::Object>::try_from(value)
            && is_window_receiver(scope, window)
        {
            return Ok(Self(window));
        }
        Err(webidl::WebIdlError::cannot_convert(context, "Window"))
    }
}

/// EventTarget conversion honors the brand layer's registered native proxies.
pub(in crate::context_bootstrap) struct EventTargetReference<'s>(v8::Local<'s, v8::Object>);

impl<'s> EventTargetReference<'s> {
    pub(in crate::context_bootstrap) fn into_value(self) -> v8::Local<'s, v8::Value> {
        self.0.into()
    }
}

impl<'s> webidl::WebIdlConverter<'s> for EventTargetReference<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let target = webidl::convert::<v8::Local<'s, v8::Object>>(scope, value, context)?;
        if !web_api_interfaces::EventTarget::is_instance(scope, target) {
            return Err(webidl::WebIdlError::cannot_convert(context, "EventTarget"));
        }
        Ok(Self(target))
    }
}
