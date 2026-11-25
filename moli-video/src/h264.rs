use anyhow::{Result, anyhow, ensure};
use moli_image::{ImagePixels, RgbaImage};
use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVCodecParameters, AVPacket};
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;
use std::sync::Arc;

#[cfg(all(feature = "molio", target_os = "macos"))]
mod macos;

pub(super) const MICROSECONDS: ffi::AVRational = ffi::AVRational {
    num: 1,
    den: 1_000_000,
};
const MAX_FRAME_PIXELS: i64 = 4096 * 2304;
const FRAME_BUFFER_ALIGNMENT: usize = 64;

pub struct VideoFrame {
    pub timestamp: f64,
    pub duration: f64,
    pub image: ImagePixels,
}

pub(super) struct H264Decoder {
    codec: AVCodecContext,
    scaler: SwsContext,
    #[cfg(all(feature = "molio", target_os = "macos"))]
    surfaces: Box<macos::DecodeSurfaces>,
}

pub(super) enum Decoded {
    Frame(AVFrame),
    NeedsPacket,
    EndOfStream,
}

impl H264Decoder {
    pub(super) fn new(parameters: &AVCodecParameters, low_latency: bool) -> Result<Self> {
        let decoder = AVCodec::find_decoder(ffi::AV_CODEC_ID_H264)
            .ok_or_else(|| anyhow!("FFmpeg H.264 decoder is unavailable"))?;
        let mut codec = AVCodecContext::new(&decoder);
        codec.apply_codecpar(parameters)?;
        codec.set_pkt_timebase(MICROSECONDS);
        codec.set_max_pixels(MAX_FRAME_PIXELS);
        let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get());
        codec.set_thread_count(cores.clamp(1, 4) as i32);
        let mut thread_type = ffi::FF_THREAD_SLICE;
        if !low_latency {
            thread_type |= ffi::FF_THREAD_FRAME;
        }
        codec.set_thread_type(thread_type as i32);
        let scaler = SwsContext::for_frames().ok_or_else(|| anyhow!("allocate video scaler"))?;
        #[cfg(all(feature = "molio", target_os = "macos"))]
        let surfaces = macos::DecodeSurfaces::install(&mut codec);
        let mut decoder = Self {
            codec,
            scaler,
            #[cfg(all(feature = "molio", target_os = "macos"))]
            surfaces,
        };
        decoder.codec.open(None)?;
        Ok(decoder)
    }

    pub(super) fn receive(&mut self) -> Result<Decoded> {
        match self.codec.receive_frame() {
            Ok(frame) => Ok(Decoded::Frame(frame)),
            Err(RsmpegError::DecoderDrainError) => Ok(Decoded::NeedsPacket),
            Err(RsmpegError::DecoderFlushedError) => Ok(Decoded::EndOfStream),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn send(&mut self, packet: Option<&AVPacket>) -> Result<()> {
        // The caller sends only after receive reports EAGAIN. FFmpeg guarantees
        // that both ends cannot return EAGAIN at the same time.
        self.codec.send_packet(packet)?;
        Ok(())
    }

    pub(super) fn reset(&mut self) {
        self.codec.flush_buffers();
    }

    pub(super) fn frame(
        &mut self,
        frame: AVFrame,
        origin: i64,
        next_timestamp: &mut f64,
    ) -> Result<VideoFrame> {
        let timestamp = if frame.best_effort_timestamp == ffi::AV_NOPTS_VALUE {
            *next_timestamp
        } else {
            super::mp4::as_seconds(frame.best_effort_timestamp.saturating_sub(origin))
        };
        let duration = super::mp4::as_seconds(frame.duration.max(0));
        *next_timestamp = timestamp + duration;
        Ok(VideoFrame {
            timestamp,
            duration,
            image: self.frame_pixels(frame)?,
        })
    }

    fn frame_pixels(&mut self, frame: AVFrame) -> Result<ImagePixels> {
        #[cfg(all(feature = "molio", target_os = "macos"))]
        if let Some(surface) = self.surfaces.describe(&frame) {
            return Ok(surface.into_image(frame));
        }
        Self::convert_pixels(&mut self.scaler, &frame).map(Into::into)
    }

    fn convert_pixels(scaler: &mut SwsContext, frame: &AVFrame) -> Result<Arc<RgbaImage>> {
        let (width, height) = (frame.width, frame.height);
        ensure!(
            width > 0 && height > 0 && i64::from(width) * i64::from(height) <= MAX_FRAME_PIXELS,
            "unsupported video dimensions {width}x{height}"
        );
        ensure!(!frame.data[0].is_null(), "missing FFmpeg video plane");

        let row_bytes = width as usize * 4;
        let len = row_bytes * height as usize;
        let aligned_stride = row_bytes.is_multiple_of(FRAME_BUFFER_ALIGNMENT);
        let mut pixels = if aligned_stride {
            // Match the alignment and trailing space of FFmpeg's packed frame allocation.
            let padded_rows = (height as usize).next_multiple_of(32);
            vec![0; row_bytes * padded_rows + FRAME_BUFFER_ALIGNMENT]
        } else {
            Vec::with_capacity(len)
        };

        let mut rgba = AVFrame::new();
        rgba.set_width(width);
        rgba.set_height(height);
        rgba.set_format(ffi::AV_PIX_FMT_RGBA);
        rgba.set_colorspace(ffi::AVCOL_SPC_RGB);
        rgba.set_color_range(ffi::AVCOL_RANGE_JPEG);
        rgba.set_color_primaries(frame.color_primaries);
        rgba.set_color_trc(frame.color_trc);
        let write_directly =
            aligned_stride && pixels.as_ptr().align_offset(FRAME_BUFFER_ALIGNMENT) == 0;
        if write_directly {
            // The frame borrows an aligned, padded plane from pixels. The
            // synchronous swscale call writes to it without taking ownership.
            // pixels outlives rgba and is not resized while borrowed.
            unsafe {
                let dst = &mut *rgba.as_mut_ptr();
                dst.data[0] = pixels.as_mut_ptr();
                dst.linesize[0] = row_bytes as i32;
            }
        }
        scaler.convert_frame(frame, &mut rgba)?;

        if write_directly {
            drop(rgba);
            pixels.truncate(len);
        } else {
            let stride = usize::try_from(rgba.linesize[0])?;
            ensure!(
                !rgba.data[0].is_null() && stride >= row_bytes,
                "invalid FFmpeg RGBA plane"
            );
            pixels.clear();
            for row in 0..height as usize {
                // FFmpeg keeps the plane live until rgba is dropped. Each
                // row contains at least width * 4 bytes.
                let source = unsafe {
                    std::slice::from_raw_parts(rgba.data[0].add(row * stride), row_bytes)
                };
                pixels.extend_from_slice(source);
            }
        }
        Ok(Arc::new(RgbaImage::try_new(
            width as u32,
            height as u32,
            pixels,
        )?))
    }
}
