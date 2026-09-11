use moli_encoding::{XhrResponseDecoder, XhrResponseTextKind, encoding_for_label};
use moli_web_mime::{effective_response_mime_essence, mime_charset, response_content_type};

use super::*;

pub(crate) fn xhr_response_text_decoder(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    headers: &[(String, String)],
) -> XhrResponseDecoder {
    let override_mime = xhr_state_string_property(scope, xhr, XHR_OVERRIDE_MIME_TYPE_SLOT);
    // A MIME-only override must retain the response charset. Blink also
    // ignores an empty override charset, but an unknown nonempty label
    // replaces the transport label and lets content/default detection run.
    let charset = override_mime
        .as_deref()
        .and_then(mime_charset)
        .filter(|label| !label.is_empty())
        .or_else(|| {
            response_content_type(headers)
                .as_deref()
                .and_then(mime_charset)
        });
    let mime = effective_response_mime_essence(headers, override_mime.as_deref())
        .unwrap_or_else(|| "text/xml".to_owned());
    let kind = match xhr_response_type(scope, xhr) {
        XmlHttpRequestResponseType::Document if mime == "text/html" => XhrResponseTextKind::Html,
        XmlHttpRequestResponseType::Default | XmlHttpRequestResponseType::Document
            if matches!(mime.as_str(), "text/xml" | "application/xml")
                || mime.ends_with("+xml") =>
        {
            XhrResponseTextKind::Xml
        }
        _ => XhrResponseTextKind::Text,
    };
    XhrResponseDecoder::new(kind, charset.as_deref().and_then(encoding_for_label))
}
