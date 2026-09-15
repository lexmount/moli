use super::entries::{headers_entries_if_present, normalized_header_entry_or_throw};
use crate::webidl;

pub(in crate::network_host) fn headers_entries_from_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init_arg: v8::Local<'s, v8::Value>,
) -> Result<Vec<(String, String)>, webidl::WebIdlError> {
    if init_arg.is_undefined() {
        return Ok(vec![]);
    }
    if init_arg.is_null() {
        return Err(webidl::WebIdlError::custom_message(
            "Headers initializer must be an object",
        ));
    }

    let Ok(init_obj) = v8::Local::<v8::Object>::try_from(init_arg) else {
        return Err(webidl::WebIdlError::custom_message(
            "Headers initializer must be an object",
        ));
    };

    // Convert every inner sequence before validating or normalizing any header.
    if let Some(sequence) = webidl::convert_optional_sequence::<webidl::Sequence<webidl::ByteString>>(
        scope,
        init_arg,
        webidl::Context::argument("Headers", 1),
        &webidl::StringOptions::default(),
    )? {
        let mut entries = Vec::with_capacity(sequence.0.len());
        for pair in sequence.0 {
            let [key, value] = <[_; 2]>::try_from(pair.0).map_err(|_| {
                webidl::WebIdlError::custom_message(
                    "Headers sequence initializer pairs must have length 2",
                )
            })?;
            let entry = normalized_header_entry_or_throw(scope, key.into(), value.into())
                .ok_or_else(|| {
                    webidl::WebIdlError::custom_message(
                        "Headers initializer contains an invalid header",
                    )
                })?;
            entries.push(entry);
        }
        return Ok(entries);
    }

    if let Some(entries) = headers_entries_if_present(scope, init_obj) {
        return Ok(entries);
    }

    let record = webidl::convert::<webidl::Record<webidl::ByteString, webidl::ByteString>>(
        scope,
        init_obj.into(),
        webidl::Context::argument("Headers", 1),
    )?;
    let mut entries = Vec::with_capacity(record.0.len());
    for (key, value) in record.0 {
        let Some(entry) = normalized_header_entry_or_throw(scope, key.into(), value.into()) else {
            return Err(webidl::WebIdlError::custom_message(
                "Headers initializer contains an invalid header",
            ));
        };
        entries.push(entry);
    }
    Ok(entries)
}
