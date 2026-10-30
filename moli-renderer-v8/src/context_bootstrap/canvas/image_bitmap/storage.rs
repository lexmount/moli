use super::*;
use crate::structured_clone::HOST_OBJECT_TAG_IMAGE_BITMAP;

pub(in crate::context_bootstrap) fn write_image_bitmap_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bitmap: v8::Local<'s, v8::Object>,
    serializer: &dyn v8::ValueSerializerHelper,
) -> Option<bool> {
    let Some(data) = image_bitmap_data(scope, bitmap) else {
        let error = new_dom_exception_value(
            scope,
            "A closed ImageBitmap cannot be stored.",
            "DataCloneError",
        );
        scope.throw_exception(error);
        return None;
    };
    if data.width == 0 || data.height == 0 {
        let error = new_dom_exception_value(
            scope,
            "A closed ImageBitmap cannot be stored.",
            "DataCloneError",
        );
        scope.throw_exception(error);
        return None;
    }
    let mut bytes = vec![0; data.pixels.byte_length()];
    data.pixels.copy_contents(&mut bytes);
    let length = u32::try_from(bytes.len()).ok()?;
    serializer.write_uint32(HOST_OBJECT_TAG_IMAGE_BITMAP);
    serializer.write_uint32(data.width);
    serializer.write_uint32(data.height);
    serializer.write_uint32(u32::from(data.premultiplied));
    // Current native bitmap pixels are sRGB/unorm8. These fields retain the
    // format used by the subsequent color/precision and wire-clone payloads.
    serializer.write_uint32(0);
    serializer.write_uint32(0);
    serializer.write_uint32(length);
    serializer.write_raw_bytes(&bytes);
    Some(true)
}

pub(in crate::context_bootstrap) fn read_image_bitmap_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
) -> Option<v8::Local<'s, v8::Object>> {
    let read = || {
        let mut n = 0;
        deserializer.read_uint32(&mut n).then_some(n)
    };
    let width = read()?;
    let height = read()?;
    let premultiplied = read()?;
    if width == 0 || height == 0 || premultiplied > 1 || read()? != 0 || read()? != 0 {
        return None;
    }
    let length = read()? as usize;
    if (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?
        != length
    {
        return None;
    }
    let bytes = deserializer.read_raw_bytes(length)?.to_vec();
    let pixels = super::super::super::image_data::new_uint8_clamped_array_from_bytes(scope, bytes)?;
    build_image_bitmap_from_data(
        scope,
        BitmapData {
            pixels,
            width,
            height,
            premultiplied: premultiplied != 0,
        },
    )
}
