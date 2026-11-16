//! Realm exposure around the shared encoded-frame value implementation.

pub(crate) use moli_webapi::rtc::encoded_frames::{
    FrameKind, audio_constructor, install, video_constructor, write_clone,
};

pub(crate) fn read_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
    kind: FrameKind,
) -> Option<v8::Local<'s, v8::Object>> {
    if !super::super::exposed_interfaces::is_realm_interface_exposed(scope, kind.name()) {
        return None;
    }
    moli_webapi::rtc::encoded_frames::read_clone(scope, deserializer, kind)
}

#[cfg(test)]
pub(crate) fn native_frame_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: FrameKind,
    data: &[u8],
    metadata: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    super::super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, kind.name())
        .expect("test frame intrinsic constructor");
    let frame_type = Some(crate::util::v8str(scope, "key"));
    moli_webapi::rtc::encoded_frames::build_native_frame(scope, kind, data, metadata, frame_type)
        .expect("native seed frame")
}
