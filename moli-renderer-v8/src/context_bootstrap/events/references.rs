use crate::context_bootstrap::file_api::is_branded_data_transfer_object;
use crate::webidl;

/// An interface-valued event dictionary member uses native identity, without
/// consulting author properties or unwrapping author-created proxies.
pub(super) struct DataTransferReference<'s>(pub(super) v8::Local<'s, v8::Object>);

impl<'s> webidl::WebIdlConverter<'s> for DataTransferReference<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let object = webidl::convert::<v8::Local<'s, v8::Object>>(scope, value, context)?;
        if !is_branded_data_transfer_object(scope, object) {
            return Err(webidl::WebIdlError::cannot_convert(context, "DataTransfer"));
        }
        Ok(Self(object))
    }
}
