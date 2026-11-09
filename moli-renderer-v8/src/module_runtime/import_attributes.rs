use super::ModuleAttributesKey;

pub(crate) fn module_request_attributes(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::ModuleRequest>,
) -> ModuleAttributesKey {
    decode_import_attributes(scope, request.get_import_attributes(), 3)
}

pub(crate) fn import_attributes_key(
    scope: &mut v8::PinScope<'_, '_>,
    attributes: v8::Local<'_, v8::FixedArray>,
) -> ModuleAttributesKey {
    decode_import_attributes(scope, attributes, 2)
}

fn decode_import_attributes(
    scope: &mut v8::PinScope<'_, '_>,
    attributes: v8::Local<'_, v8::FixedArray>,
    stride: usize,
) -> ModuleAttributesKey {
    let mut pairs = Vec::with_capacity(attributes.length() / stride);
    let mut index = 0;
    while index + 1 < attributes.length() {
        let key = attributes
            .get(scope, index)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        let value = attributes
            .get(scope, index + 1)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        if let (Some(key), Some(value)) = (key, value) {
            pairs.push((key, value));
        }
        index += stride;
    }
    ModuleAttributesKey::from_pairs(pairs)
}
