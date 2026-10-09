//! Borrowed BufferSource conversion for algorithms that copy after dictionary conversion.

use crate::{Context, WebIdlConverter, WebIdlError};
use std::sync::atomic::{AtomicU8, Ordering};

/// `[AllowShared] BufferSource`, retaining the native buffer identity instead
/// of copying during conversion. Resizable and growable buffers require the
/// separate `[AllowResizable]` annotation and are rejected here.
pub enum AllowSharedBufferSource<'s> {
    Buffer(v8::Local<'s, v8::ArrayBuffer>),
    Shared(v8::Local<'s, v8::SharedArrayBuffer>),
    View(v8::Local<'s, v8::ArrayBufferView>),
}

/// A fixed-length, non-shared `BufferSource`. Keep the native identity until
/// the consuming algorithm reads its bytes; conversion does not call author
/// getters or unwrap author Proxies.
pub struct NonSharedBufferSource<'s>(AllowSharedBufferSource<'s>);

impl<'s> WebIdlConverter<'s> for NonSharedBufferSource<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let source = AllowSharedBufferSource::convert(scope, value, context, options)?;
        if matches!(&source, AllowSharedBufferSource::Shared(_))
            || source
                .backing_store(scope)
                .is_some_and(|backing| backing.is_shared())
        {
            return Err(WebIdlError::cannot_convert(
                context,
                "non-shared BufferSource",
            ));
        }
        // A detached resizable ArrayBuffer retains its resizable identity,
        // even when its replacement empty backing store is fixed-length.
        let resizable = match &source {
            AllowSharedBufferSource::Buffer(buffer) => buffer.is_resizable_by_user_javascript(),
            AllowSharedBufferSource::View(view) => view
                .buffer(scope)
                .is_some_and(|buffer| buffer.is_resizable_by_user_javascript()),
            AllowSharedBufferSource::Shared(_) => unreachable!("shared sources were rejected"),
        };
        if resizable {
            return Err(WebIdlError::cannot_convert(
                context,
                "non-resizable BufferSource",
            ));
        }
        Ok(Self(source))
    }
}

impl NonSharedBufferSource<'_> {
    pub fn to_vec(&self, scope: &mut v8::PinScope<'_, '_>) -> Vec<u8> {
        self.0.to_vec(scope)
    }
}

impl<'s> WebIdlConverter<'s> for AllowSharedBufferSource<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let source = if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
            Self::Buffer(buffer)
        } else if let Ok(buffer) = v8::Local::<v8::SharedArrayBuffer>::try_from(value) {
            Self::Shared(buffer)
        } else if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
            Self::View(view)
        } else {
            return Err(WebIdlError::cannot_convert(context, "BufferSource"));
        };
        if source
            .backing_store(scope)
            .is_some_and(|backing| backing.is_resizable_by_user_javascript())
        {
            return Err(WebIdlError::cannot_convert(
                context,
                "non-resizable BufferSource",
            ));
        }
        Ok(source)
    }
}

impl AllowSharedBufferSource<'_> {
    pub fn byte_length(&self) -> usize {
        match self {
            Self::Buffer(buffer) => buffer.byte_length(),
            Self::Shared(buffer) => buffer.byte_length(),
            Self::View(view) => view.byte_length(),
        }
    }

    fn byte_offset(&self) -> usize {
        match self {
            Self::View(view) => view.byte_offset(),
            _ => 0,
        }
    }

    fn backing_store(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<v8::SharedRef<v8::BackingStore>> {
        match self {
            Self::Buffer(buffer) => Some(buffer.get_backing_store()),
            Self::Shared(buffer) => Some(buffer.get_backing_store()),
            Self::View(view) => view.buffer(scope).map(|buffer| buffer.get_backing_store()),
        }
    }

    pub fn to_vec(&self, scope: &mut v8::PinScope<'_, '_>) -> Vec<u8> {
        let len = self.byte_length();
        if len == 0 {
            return Vec::new();
        }
        let backing = self
            .backing_store(scope)
            .expect("BufferSource has native backing");
        let offset = self.byte_offset();
        if backing.is_shared() {
            // SAFETY: fixed shared storage is retained by `backing`, each byte
            // is aligned for AtomicU8, and the view bounds were checked by V8.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    backing
                        .data()
                        .expect("nonempty backing")
                        .as_ptr()
                        .cast::<AtomicU8>()
                        .add(offset),
                    len,
                )
            };
            bytes
                .iter()
                .map(|byte| byte.load(Ordering::Relaxed))
                .collect()
        } else {
            backing[offset..offset + len]
                .iter()
                .map(|byte| byte.get())
                .collect()
        }
    }

    /// Copy into the native view's byte range, preserving surrounding bytes.
    pub fn write_bytes(&self, scope: &mut v8::PinScope<'_, '_>, bytes: &[u8]) -> bool {
        if bytes.len() > self.byte_length() {
            return false;
        }
        if bytes.is_empty() {
            return true;
        }
        let backing = self
            .backing_store(scope)
            .expect("BufferSource has native backing");
        let offset = self.byte_offset();
        if backing.is_shared() {
            // SAFETY: fixed shared storage is retained and V8 validated the
            // view range; byte stores are unordered as required for BufferSource.
            let destination = unsafe {
                std::slice::from_raw_parts(
                    backing
                        .data()
                        .expect("nonempty backing")
                        .as_ptr()
                        .cast::<AtomicU8>()
                        .add(offset),
                    bytes.len(),
                )
            };
            for (cell, byte) in destination.iter().zip(bytes) {
                cell.store(*byte, Ordering::Relaxed);
            }
        } else {
            for (cell, byte) in backing[offset..offset + bytes.len()].iter().zip(bytes) {
                cell.set(*byte);
            }
        }
        true
    }
}

impl<'s> WebIdlConverter<'s> for v8::Local<'s, v8::ArrayBuffer> {
    type Options = ();

    fn convert(
        _scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let buffer = Self::try_from(value)
            .map_err(|_| WebIdlError::cannot_convert(context, "ArrayBuffer"))?;
        if buffer.is_resizable_by_user_javascript() {
            return Err(WebIdlError::cannot_convert(
                context,
                "non-resizable ArrayBuffer",
            ));
        }
        Ok(buffer)
    }
}
