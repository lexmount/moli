use std::{ffi::c_void, ptr::NonNull};

use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use yuv::{YuvChromaSubsampling, YuvPlanarImageMut};

#[repr(C)]
#[derive(Default)]
struct NativeOutput {
    data: *const u8,
    len: usize,
    pts: i64,
    dts: i64,
    keyframe: i32,
}

unsafe extern "C" {
    fn moli_x264_open(width: i32, height: i32, threads: i32) -> *mut c_void;
    fn moli_x264_headers(
        encoder: *mut c_void,
        sps: *mut *const u8,
        sps_len: *mut i32,
        pps: *mut *const u8,
        pps_len: *mut i32,
    ) -> i32;
    fn moli_x264_encode(
        encoder: *mut c_void,
        y: *const u8,
        u: *const u8,
        v: *const u8,
        width: i32,
        pts: i64,
        idr: i32,
        output: *mut NativeOutput,
    ) -> i32;
    fn moli_x264_delayed(encoder: *mut c_void) -> i32;
    fn moli_x264_close(encoder: *mut c_void);
}

pub(super) struct Encoder {
    native: NonNull<c_void>,
    pub planes: YuvPlanarImageMut<'static, u8>,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

pub(super) struct EncodedFrame {
    pub pts: u64,
    pub keyframe: bool,
    pub bytes: Bytes,
}

impl Encoder {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
        // SAFETY: dimensions were validated by VideoWriter; the returned handle
        // is uniquely owned and closed by Drop on the encoding thread.
        let native =
            unsafe { moli_x264_open(width as i32, height as i32, if cpus < 4 { 1 } else { 2 }) };
        let native = NonNull::new(native).context("could not initialize x264 CPU encoder")?;
        let mut encoder = Self {
            native,
            planes: YuvPlanarImageMut::alloc(width, height, YuvChromaSubsampling::Yuv420),
            sps: Vec::new(),
            pps: Vec::new(),
        };
        let (mut sps, mut pps) = (std::ptr::null(), std::ptr::null());
        let (mut sps_len, mut pps_len) = (0, 0);
        // SAFETY: all output pointers are writable locals. Copy the borrowed
        // parameter sets before any subsequent encoder call invalidates them.
        let status = unsafe {
            moli_x264_headers(
                native.as_ptr(),
                &mut sps,
                &mut sps_len,
                &mut pps,
                &mut pps_len,
            )
        };
        ensure!(
            status == 0 && sps_len > 0 && pps_len > 0,
            "x264 did not produce SPS/PPS"
        );
        // SAFETY: the successful bridge call supplies valid parameter-set slices.
        unsafe {
            encoder.sps = std::slice::from_raw_parts(sps, sps_len as usize).to_vec();
            encoder.pps = std::slice::from_raw_parts(pps, pps_len as usize).to_vec();
        }
        Ok(encoder)
    }

    pub fn encode(&mut self, pts: u64, idr: bool) -> Result<Option<EncodedFrame>> {
        self.call(pts, idr, false)
    }

    pub fn drain(&mut self) -> Result<Option<EncodedFrame>> {
        // SAFETY: exclusively owned live encoder handle.
        if unsafe { moli_x264_delayed(self.native.as_ptr()) } == 0 {
            return Ok(None);
        }
        self.call(0, false, true)
    }

    fn call(&mut self, pts: u64, idr: bool, drain: bool) -> Result<Option<EncodedFrame>> {
        let mut output = NativeOutput::default();
        let pts = i64::try_from(pts).context("video timestamp exceeds x264 range")?;
        // SAFETY: the I420 allocation owns validated full-resolution Y and
        // half-resolution chroma planes, alive for this synchronous call. x264
        // copies input into its own frame storage. A null Y requests flushing.
        let size = unsafe {
            moli_x264_encode(
                self.native.as_ptr(),
                if drain {
                    std::ptr::null()
                } else {
                    self.planes.y_plane.borrow().as_ptr()
                },
                self.planes.u_plane.borrow().as_ptr(),
                self.planes.v_plane.borrow().as_ptr(),
                self.planes.width as i32,
                pts,
                i32::from(idr),
                &mut output,
            )
        };
        ensure!(size >= 0, "x264 frame encoding failed");
        if size == 0 {
            return Ok(None);
        }
        ensure!(
            output.pts >= 0 && output.pts == output.dts,
            "unexpected reordered x264 timestamp"
        );
        // SAFETY: all returned NALs form one contiguous access unit, valid
        // until the next encoder call. Retain an owned copy for the muxer.
        let bytes =
            unsafe { Bytes::copy_from_slice(std::slice::from_raw_parts(output.data, output.len)) };
        Ok(Some(EncodedFrame {
            pts: output.pts as u64,
            keyframe: output.keyframe != 0,
            bytes,
        }))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: exactly one close of the uniquely owned encoder handle.
        unsafe { moli_x264_close(self.native.as_ptr()) }
    }
}
