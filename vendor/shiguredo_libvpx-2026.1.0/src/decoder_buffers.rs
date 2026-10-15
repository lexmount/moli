//! Owned, fallible external VP9 frame buffers. The codec releases every buffer
//! before Decoder drops this callback state.

use crate::sys;
use std::{collections::HashMap, ffi::c_void, sync::Mutex};

pub(crate) struct DecoderBuffers {
    state: Mutex<State>,
}

struct State {
    maximum_frame_bytes: usize,
    maximum_retained_bytes: usize,
    retained_bytes: usize,
    allocations: HashMap<usize, Vec<u8>>,
}

impl DecoderBuffers {
    pub(crate) fn increase_limit(&self, maximum: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.maximum_frame_bytes = state.maximum_frame_bytes.max(maximum);
            state.maximum_retained_bytes = state.maximum_frame_bytes.saturating_mul(4);
        }
    }

    pub(crate) fn new(maximum_frame_bytes: usize) -> Self {
        Self {
            state: Mutex::new(State {
                maximum_frame_bytes,
                maximum_retained_bytes: maximum_frame_bytes.saturating_mul(4),
                retained_bytes: 0,
                allocations: HashMap::new(),
            }),
        }
    }

    pub(crate) unsafe fn install(&self, context: &mut sys::vpx_codec_ctx) -> sys::vpx_codec_err_t {
        // The owner stores this object in a Box, keeping userdata stable when
        // Decoder moves. Mutex also protects callbacks from codec workers.
        unsafe {
            sys::vpx_codec_set_frame_buffer_functions(
                context,
                Some(allocate),
                Some(release),
                (self as *const Self).cast_mut().cast(),
            )
        }
    }
}

unsafe extern "C" fn allocate(
    private: *mut c_void,
    minimum: usize,
    buffer: *mut sys::vpx_codec_frame_buffer_t,
) -> std::ffi::c_int {
    let Some(owner) = (unsafe { private.cast::<DecoderBuffers>().as_ref() }) else {
        return -1;
    };
    let Some(buffer) = (unsafe { buffer.as_mut() }) else {
        return -1;
    };
    let Ok(mut state) = owner.state.lock() else {
        return -1;
    };
    let Some(total) = state.retained_bytes.checked_add(minimum) else {
        return -1;
    };
    if minimum == 0 || minimum > state.maximum_frame_bytes || total > state.maximum_retained_bytes {
        return -1;
    }
    if state.allocations.try_reserve(1).is_err() {
        return -1;
    }
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(minimum).is_err() {
        return -1;
    }
    bytes.resize(minimum, 0);
    buffer.data = bytes.as_mut_ptr();
    buffer.size = minimum;
    buffer.priv_ = buffer.data.cast();
    state.allocations.insert(buffer.data as usize, bytes);
    state.retained_bytes = total;
    0
}

unsafe extern "C" fn release(
    private: *mut c_void,
    buffer: *mut sys::vpx_codec_frame_buffer_t,
) -> std::ffi::c_int {
    let Some(owner) = (unsafe { private.cast::<DecoderBuffers>().as_ref() }) else {
        return -1;
    };
    let Some(buffer) = (unsafe { buffer.as_mut() }) else {
        return -1;
    };
    let Ok(mut state) = owner.state.lock() else {
        return -1;
    };
    let Some(bytes) = state.allocations.remove(&(buffer.priv_ as usize)) else {
        return -1;
    };
    state.retained_bytes -= bytes.len();
    buffer.data = std::ptr::null_mut();
    buffer.size = 0;
    buffer.priv_ = std::ptr::null_mut();
    0
}
