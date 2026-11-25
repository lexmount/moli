use std::{
    collections::HashMap,
    ffi::c_void,
    ptr,
    sync::{Arc, Mutex, Weak},
};

use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_video::{
    pixel_buffer::{CVPixelBuffer, CVPixelBufferKeys, kCVPixelFormatType_420YpCbCr8Planar},
    pixel_buffer_pool::CVPixelBufferPool,
};
use moli_image::{ImagePixels, NativeVideoFrame};
use rsmpeg::{avcodec::AVCodecContext, avutil::AVFrame, ffi, swscale::SwsContext};

use super::H264Decoder;

#[derive(Default)]
pub(super) struct DecodeSurfaces {
    buffers: Mutex<Buffers>,
    scaler: Arc<Mutex<Option<SwsContext>>>,
}

#[derive(Default)]
struct Buffers {
    pool: Option<((usize, usize), CVPixelBufferPool)>,
    owners: HashMap<usize, Weak<LockedBuffer>>,
}

// Access to the CoreVideo pool is serialized across FFmpeg's decode threads.
unsafe impl Send for Buffers {}

struct LockedBuffer(CVPixelBuffer);

// Each plane keeps its buffer locked until FFmpeg releases the last AVBufferRef.
// The FFmpeg buffer API controls exclusive writes and shared reads.
unsafe impl Send for LockedBuffer {}
unsafe impl Sync for LockedBuffer {}

impl Drop for LockedBuffer {
    fn drop(&mut self) {
        self.0.unlock_base_address(0);
    }
}

pub(super) struct DecodedSurface {
    buffer: CVPixelBuffer,
    visible: [u32; 4],
    conversion: [[f32; 4]; 3],
    chroma_offset: [f32; 2],
    scaler: Arc<Mutex<Option<SwsContext>>>,
}

impl DecodeSurfaces {
    pub(super) fn install(codec: &mut AVCodecContext) -> Box<Self> {
        let mut surfaces = Box::<Self>::default();
        // H264Decoder drops its codec before dropping this allocation.
        unsafe {
            let context = &mut *codec.as_mut_ptr();
            context.opaque = ptr::from_mut(surfaces.as_mut()).cast();
            context.get_buffer2 = Some(get_buffer);
        }
        surfaces
    }

    pub(super) fn describe(&self, frame: &AVFrame) -> Option<DecodedSurface> {
        if !is_planar_420(frame.format)
            || frame.flags & ffi::AV_FRAME_FLAG_INTERLACED as i32 != 0
            || frame.width <= 0
            || frame.height <= 0
            || frame.crop_left != 0
            || frame.crop_right != 0
            || frame.crop_top != 0
            || frame.crop_bottom != 0
        {
            return None;
        }
        let first_plane = ptr::NonNull::new(frame.buf[0])?;
        // Each video plane has an AVBufferRef beginning at the allocated
        // address, even when FFmpeg advanced data[0] to apply a crop.
        let key = unsafe { first_plane.as_ref().data } as usize;
        let owner = self.buffers.lock().ok()?.owners.get(&key)?.upgrade()?;
        let buffer = &owner.0;
        let stride = buffer.get_bytes_per_row_of_plane(0);
        let offset = (frame.data[0] as usize).checked_sub(key)?;
        let (x, y) = (offset % stride, offset / stride);
        let width = frame.width as usize;
        let height = frame.height as usize;
        if !x.is_multiple_of(2)
            || !y.is_multiple_of(2)
            || x + width > buffer.get_width_of_plane(0)
            || y + height > buffer.get_height_of_plane(0)
            || (1..3).any(|plane| {
                // The decoded AVFrame still holds the lock for each plane.
                let expected = unsafe { buffer.get_base_address_of_plane(plane) } as usize
                    + y / 2 * buffer.get_bytes_per_row_of_plane(plane)
                    + x / 2;
                frame.data[plane] as usize != expected
            })
        {
            return None;
        }
        Some(DecodedSurface {
            buffer: buffer.clone(),
            visible: [x as u32, y as u32, width as u32, height as u32],
            conversion: conversion(frame)?,
            chroma_offset: chroma_offset(frame.chroma_location)?,
            scaler: Arc::clone(&self.scaler),
        })
    }

    fn allocate(
        &self,
        width: usize,
        height: usize,
        alignment: [usize; 3],
    ) -> Option<[ptr::NonNull<ffi::AVBufferRef>; 3]> {
        let mut buffers = self.buffers.lock().ok()?;
        let image = buffers.pixel_buffer(width, height)?;
        if image.lock_base_address(0) != 0 {
            return None;
        }
        let owner = Arc::new(LockedBuffer(image));
        let image = &owner.0;
        let mut planes = [(ptr::null_mut(), 0); 3];
        for (index, (base, bytes)) in planes.iter_mut().enumerate() {
            // The lock belongs to owner until the last AVBufferRef is released.
            *base = unsafe { image.get_base_address_of_plane(index) }.cast::<u8>();
            let stride = image.get_bytes_per_row_of_plane(index);
            if base.is_null()
                || base.align_offset(64) != 0
                || !stride.is_multiple_of(alignment[index])
            {
                return None;
            }
            *bytes = stride * image.get_height_of_plane(index);
        }
        let mut frame_buffers = [ptr::null_mut(); 3];
        for (index, (base, bytes)) in planes.into_iter().enumerate() {
            let opaque = Arc::into_raw(Arc::clone(&owner))
                .cast_mut()
                .cast::<c_void>();
            let reference =
                unsafe { ffi::av_buffer_create(base, bytes, Some(release_buffer), opaque, 0) };
            if reference.is_null() {
                unsafe { release_buffer(opaque, base) };
                for reference in &mut frame_buffers[..index] {
                    unsafe { ffi::av_buffer_unref(reference) };
                }
                return None;
            }
            frame_buffers[index] = reference;
        }
        buffers.owners.retain(|_, owner| owner.strong_count() > 0);
        buffers
            .owners
            .insert(planes[0].0 as usize, Arc::downgrade(&owner));
        Some(frame_buffers.map(|reference| unsafe { ptr::NonNull::new_unchecked(reference) }))
    }
}

impl Buffers {
    fn pixel_buffer(&mut self, width: usize, height: usize) -> Option<CVPixelBuffer> {
        let size = (width, height);
        if self
            .pool
            .as_ref()
            .is_none_or(|(current, _)| *current != size)
        {
            let attributes = CFDictionary::from_CFType_pairs(&[
                (
                    CFString::from(CVPixelBufferKeys::Width),
                    CFNumber::from(width as i64).as_CFType(),
                ),
                (
                    CFString::from(CVPixelBufferKeys::Height),
                    CFNumber::from(height as i64).as_CFType(),
                ),
                (
                    CFString::from(CVPixelBufferKeys::PixelFormatType),
                    CFNumber::from(i64::from(kCVPixelFormatType_420YpCbCr8Planar)).as_CFType(),
                ),
                (
                    CFString::from(CVPixelBufferKeys::BytesPerRowAlignment),
                    CFNumber::from(64_i64).as_CFType(),
                ),
                (
                    CFString::from(CVPixelBufferKeys::MetalCompatibility),
                    CFBoolean::true_value().as_CFType(),
                ),
                (
                    CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
                    CFDictionary::<CFString, CFType>::from_CFType_pairs(&[]).as_CFType(),
                ),
            ]);
            self.pool = Some((size, CVPixelBufferPool::new(None, Some(&attributes)).ok()?));
        }
        self.pool.as_ref()?.1.create_pixel_buffer().ok()
    }
}

unsafe extern "C" fn get_buffer(
    context: *mut ffi::AVCodecContext,
    frame: *mut ffi::AVFrame,
    flags: i32,
) -> i32 {
    let frame_data = unsafe { &mut *frame };
    if is_planar_420(frame_data.format) && frame_data.width > 0 && frame_data.height > 0 {
        let (mut width, mut height) = (frame_data.width, frame_data.height);
        let mut alignment = [0; 8];
        unsafe {
            ffi::avcodec_align_dimensions2(context, &mut width, &mut height, alignment.as_mut_ptr())
        };
        // Leave the end-of-plane padding needed by FFmpeg's vector paths.
        let padded_height = (height as usize).next_multiple_of(32) + 32;
        let align = [alignment[0], alignment[1], alignment[2]].map(|value| value.max(1) as usize);
        let surfaces = unsafe { &*(*context).opaque.cast::<DecodeSurfaces>() };
        if let Some(planes) = surfaces.allocate(width as usize, padded_height, align) {
            for (index, reference) in planes.into_iter().enumerate() {
                // The AVBufferRef owns an Arc<LockedBuffer>.
                let owner = unsafe {
                    &*ffi::av_buffer_get_opaque(reference.as_ptr()).cast::<LockedBuffer>()
                };
                frame_data.buf[index] = reference.as_ptr();
                frame_data.data[index] = unsafe { reference.as_ref().data };
                frame_data.linesize[index] = owner.0.get_bytes_per_row_of_plane(index) as i32;
            }
            frame_data.extended_data = frame_data.data.as_mut_ptr();
            return 0;
        }
    }
    unsafe { ffi::avcodec_default_get_buffer2(context, frame, flags) }
}

unsafe extern "C" fn release_buffer(opaque: *mut c_void, _: *mut u8) {
    // av_buffer_create stored one strong ref, independent of the other planes.
    drop(unsafe { Arc::from_raw(opaque.cast::<LockedBuffer>()) });
}

impl DecodedSurface {
    pub(super) fn into_image(self, frame: AVFrame) -> ImagePixels {
        let Self {
            buffer,
            visible,
            conversion,
            chroma_offset,
            scaler,
        } = self;
        // The retained AVFrame and CVPixelBuffer both own these finalized
        // pixels. FFmpeg cannot reuse the backing surface while Metal holds it.
        let frame = unsafe {
            NativeVideoFrame::new(buffer, visible, conversion, chroma_offset, move || {
                let mut scaler = scaler.lock().ok()?;
                if scaler.is_none() {
                    *scaler = SwsContext::for_frames();
                }
                H264Decoder::convert_pixels(scaler.as_mut()?, &frame).ok()
            })
        };
        ImagePixels::Video {
            frame: Arc::new(frame),
            source: None,
        }
    }
}

fn conversion(frame: &AVFrame) -> Option<[[f32; 4]; 3]> {
    if matches!(
        frame.color_trc,
        ffi::AVCOL_TRC_SMPTE2084 | ffi::AVCOL_TRC_ARIB_STD_B67
    ) {
        return None;
    }
    let (kr, kb) = match frame.colorspace {
        ffi::AVCOL_SPC_BT709 => (0.2126, 0.0722),
        ffi::AVCOL_SPC_UNSPECIFIED | ffi::AVCOL_SPC_BT470BG | ffi::AVCOL_SPC_SMPTE170M => {
            (0.299, 0.114)
        }
        _ => return None,
    };
    let color_range = if frame.format == ffi::AV_PIX_FMT_YUVJ420P {
        ffi::AVCOL_RANGE_JPEG
    } else {
        frame.color_range
    };
    let (y, c, black) = match color_range {
        ffi::AVCOL_RANGE_JPEG => (1.0, 1.0, 0.0),
        ffi::AVCOL_RANGE_MPEG | ffi::AVCOL_RANGE_UNSPECIFIED => {
            (255.0 / 219.0, 255.0 / 224.0, 16.0 / 219.0)
        }
        _ => return None,
    };
    let kg = 1.0 - kr - kb;
    let red = 2.0 * (1.0 - kr) * c;
    let blue = 2.0 * (1.0 - kb) * c;
    let green_cb = -kb * blue / kg;
    let green_cr = -kr * red / kg;
    let chroma_mid = 128.0 / 255.0;
    Some([
        [y, 0.0, red, -black - chroma_mid * red],
        [
            y,
            green_cb,
            green_cr,
            -black - chroma_mid * (green_cb + green_cr),
        ],
        [y, blue, 0.0, -black - chroma_mid * blue],
    ])
}

fn is_planar_420(format: i32) -> bool {
    matches!(format, ffi::AV_PIX_FMT_YUV420P | ffi::AV_PIX_FMT_YUVJ420P)
}

fn chroma_offset(location: ffi::AVChromaLocation) -> Option<[f32; 2]> {
    Some(match location {
        ffi::AVCHROMA_LOC_LEFT | ffi::AVCHROMA_LOC_UNSPECIFIED => [0.25, 0.0],
        ffi::AVCHROMA_LOC_CENTER => [0.0, 0.0],
        ffi::AVCHROMA_LOC_TOP => [0.0, 0.25],
        ffi::AVCHROMA_LOC_TOPLEFT => [0.25, 0.25],
        ffi::AVCHROMA_LOC_BOTTOM => [0.0, -0.25],
        ffi::AVCHROMA_LOC_BOTTOMLEFT => [0.25, -0.25],
        _ => return None,
    })
}
