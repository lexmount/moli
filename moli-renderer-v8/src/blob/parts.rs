use super::native_blob_line_ending;
use crate::{web_api_interfaces, webidl};
use moli_file_api::{BlobLineEndings, normalize_blob_line_endings_with_native_ending};

pub(crate) struct BlobParts<'s>(Vec<BlobPart<'s>>);

enum BlobPart<'s> {
    Blob(v8::Local<'s, v8::Object>),
    Buffer(webidl::NonSharedBufferSource<'s>),
    Text(String),
}

impl<'s> webidl::WebIdlConverter<'s> for BlobPart<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && web_api_interfaces::Blob::is_instance(scope, object)
        {
            let target = moli_webapi_declare::web_api_object_target(scope, object)
                .expect("converted Blob part");
            return Ok(Self::Blob(target));
        }
        if value.is_array_buffer() || value.is_shared_array_buffer() || value.is_array_buffer_view()
        {
            return webidl::convert::<webidl::NonSharedBufferSource>(scope, value, context)
                .map(Self::Buffer);
        }
        webidl::convert::<webidl::UsvString>(scope, value, context).map(|text| Self::Text(text.0))
    }
}

impl<'s> BlobParts<'s> {
    pub(crate) fn into_bytes(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        endings: BlobLineEndings,
    ) -> Result<Vec<u8>, webidl::WebIdlError> {
        let mut bytes = Vec::new();
        for part in self.0 {
            match part {
                BlobPart::Blob(object) => {
                    let data = super::blob_bytes_from_object(scope, object).ok_or_else(|| {
                        webidl::WebIdlError::cannot_convert(
                            webidl::Context::member("BlobPart", "value"),
                            "Blob",
                        )
                    })?;
                    bytes.extend(data);
                }
                BlobPart::Buffer(source) => bytes.extend(source.to_vec(scope)),
                BlobPart::Text(text) => bytes.extend(
                    normalize_blob_line_endings_with_native_ending(
                        &text,
                        endings,
                        native_blob_line_ending(),
                    )
                    .into_bytes(),
                ),
            }
        }
        Ok(bytes)
    }
}

pub(crate) fn optional_blob_parts_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<BlobParts<'s>, webidl::WebIdlError> {
    let value = args.get(index);
    if value.is_undefined() {
        return Ok(BlobParts(Vec::new()));
    }
    let parts = webidl::convert::<webidl::Sequence<BlobPart>>(
        scope,
        value,
        webidl::Context::argument("Blob", index as usize + 1),
    )?;
    Ok(BlobParts(parts.0))
}

pub(crate) fn required_blob_parts_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<BlobParts<'s>, webidl::WebIdlError> {
    let parts = webidl::convert::<webidl::Sequence<BlobPart>>(
        scope,
        args.get(index),
        webidl::Context::argument("File", index as usize + 1),
    )?;
    Ok(BlobParts(parts.0))
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "BlobPropertyBag")]
pub(crate) struct BlobPropertyBag {
    #[webidl(converter = "enum", default = BlobLineEndingsWebIdl::Transparent)]
    endings: BlobLineEndingsWebIdl,
    #[webidl(default = String::new())]
    pub(crate) r#type: String,
}

impl BlobPropertyBag {
    pub(crate) fn line_endings(&self) -> BlobLineEndings {
        match self.endings {
            BlobLineEndingsWebIdl::Transparent => BlobLineEndings::Transparent,
            BlobLineEndingsWebIdl::Native => BlobLineEndings::Native,
        }
    }
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "EndingType")]
enum BlobLineEndingsWebIdl {
    #[default]
    Transparent,
    Native,
}
