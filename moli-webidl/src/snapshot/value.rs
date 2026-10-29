use super::*;

impl AsRef<[u8]> for crate::BufferSource {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

macro_rules! scalar_snapshot {
    ($($ty:ty),* $(,)?) => { $(impl WebIdlSnapshotValue for $ty { const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Scalar; })* };
}
scalar_snapshot!(
    String, str, &str, bool, u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64
);

impl<T: WebIdlSnapshotValue> WebIdlSnapshotValue for Option<T> {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Nullable(&T::SNAPSHOT_KIND);
}
impl<T: WebIdlSnapshotValue> WebIdlSnapshotValue for Vec<T> {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Sequence(&T::SNAPSHOT_KIND);
}
impl<T> WebIdlSnapshotValue for v8::Local<'_, T> {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Identity;
}
impl<T: WebIdlSnapshotValue> WebIdlSnapshotValue for crate::Sequence<T> {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Sequence(&T::SNAPSHOT_KIND);
}
impl<'s, T: WebApiValue<'s>> WebApiValue<'s> for crate::Sequence<T> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        self.0.to_v8_value(scope)
    }
}
impl<T: WebIdlDictionarySnapshot> WebIdlSnapshotValue for crate::Dictionary<T> {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Dictionary(&T::SNAPSHOT_SCHEMA);
}
impl<'s, T: WebIdlDictionarySerialize<'s>> WebApiValue<'s> for crate::Dictionary<T> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        self.0
            .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
            .map(Into::into)
    }
}
impl<K, V: WebIdlSnapshotValue> WebIdlSnapshotValue for crate::Record<K, V> {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Record(&V::SNAPSHOT_KIND);
}
impl<'s, K: WebApiValue<'s>, V: WebApiValue<'s>> WebApiValue<'s> for crate::Record<K, V> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let output = new_null_prototype_object(scope);
        for (key, value) in &self.0 {
            let key = v8::Local::<v8::Name>::try_from(key.to_v8_value(scope)?).ok()?;
            let value = value.to_v8_value(scope)?;
            output
                .create_data_property(scope, key, value)?
                .then_some(())?;
        }
        Some(output.into())
    }
}
macro_rules! wrapper_snapshot {
    ($($ty:ty),* $(,)?) => { $(
        impl WebIdlSnapshotValue for $ty { const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Scalar; }
        impl<'s> WebApiValue<'s> for $ty {
            fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> { self.0.to_v8_value(scope) }
        }
    )* };
}
wrapper_snapshot!(
    crate::Boolean,
    crate::Octet,
    crate::UnrestrictedDouble,
    crate::UnrestrictedFloat,
    crate::Float,
    crate::Double,
    crate::Long,
    crate::UnsignedLong,
    crate::Short,
    crate::UnsignedShort,
    crate::ClampedUnsignedShort,
    crate::LongLong,
    crate::ClampedLongLong,
    crate::UnsignedLongLong,
    crate::EnforceRangeLongLong,
    crate::DomString,
    crate::UsvString,
    crate::EnforceRangeUnsignedLong,
    crate::EnforceRangeUnsignedLongLong,
    crate::EnforceRangeLong,
    crate::EnforceRangeUnsignedShort
);
impl WebIdlSnapshotValue for crate::DomString16 {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Scalar;
}
impl<'s> WebApiValue<'s> for crate::DomString16 {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        v8::String::new_from_two_byte(scope, &self.0, v8::NewStringType::Normal).map(Into::into)
    }
}
impl WebIdlSnapshotValue for crate::BufferSource {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::BufferSource;
}
impl<'s> WebApiValue<'s> for crate::BufferSource {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        Some(buffer_from_bytes(scope, self.0.clone()))
    }
}

impl<T: crate::WebIdlEnumSnapshot> WebIdlSnapshotValue for crate::EnumValue<T> {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Scalar;
}
impl<'s, T: crate::WebIdlEnumSnapshot> WebApiValue<'s> for crate::EnumValue<T> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        crate::__private::enum_snapshot_value(scope, &self.0)
    }
}
impl WebIdlSnapshotValue for crate::InterfaceObject<'_> {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Identity;
}
impl<'s> WebApiValue<'s> for crate::InterfaceObject<'s> {
    fn to_v8_value(&self, _: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        Some(self.0.into())
    }
}
impl WebIdlSnapshotValue for crate::ByteString {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Scalar;
}
impl<'s> WebApiValue<'s> for crate::ByteString {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        v8::String::new_from_one_byte(scope, &self.0, v8::NewStringType::Normal).map(Into::into)
    }
}
