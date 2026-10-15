//! DOM-neutral WebM resources and timestamp-selected software video frames.
//!
//! Only compressed packets and the current decoded frame are retained. Seeking
//! backwards resets the codec and replays its reference frames, rather than
//! returning a cached first frame for every playback position.

use matroska_demuxer::{Frame, MatroskaFile, TrackType};
use shiguredo_libvpx::{Decoder, DecoderCodec, DecoderConfig};
use std::{io::Cursor, sync::Arc};

pub use matroska_demuxer::{MatrixCoefficients, Primaries, Range, TransferCharacteristics};

mod metadata;
pub use metadata::{AudioMetadataError, ogg_vorbis_duration};

pub const MAX_ENCODED_VIDEO_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_VIDEO_FRAME_PIXELS: usize = 16 * 1024 * 1024;
const MAX_VIDEO_PACKETS: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("the media resource exceeds the video allocation limit")]
    Limit,
    #[error("unsupported video codec or pixel format")]
    Unsupported,
    #[error("the video contains no displayable frame")]
    Empty,
    #[error("video decoding was cancelled")]
    Cancelled,
    #[error("invalid WebM resource: {0}")]
    Container(#[from] matroska_demuxer::DemuxError),
    #[error("video decode failed: {0}")]
    Decode(#[from] shiguredo_libvpx::Error),
}

/// Native resource owners can cancel work and account retained codec storage
/// without passing DOM state or VM handles into the decoder.
pub trait VideoDecodeControl {
    fn is_cancelled(&self) -> bool;
    fn admit_retained_bytes(&mut self, bytes: usize) -> Result<(), VideoError>;
}

struct UnmanagedControl;
impl VideoDecodeControl for UnmanagedControl {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn admit_retained_bytes(&mut self, _: usize) -> Result<(), VideoError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VideoColorSpace {
    pub matrix: MatrixCoefficients,
    pub primaries: Primaries,
    pub transfer: TransferCharacteristics,
    pub full_range: Option<bool>,
    pub profile_known: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration: Option<f64>,
    pub color_space: VideoColorSpace,
}

#[derive(Debug)]
pub struct DecodedVideoFrame {
    pub width: u32,
    pub height: u32,
    pub timestamp: f64,
    pub color_space: VideoColorSpace,
    /// Dense 8-bit I420, Y followed by U and V; chroma dimensions round up.
    pub bytes: Vec<u8>,
}

struct Packet {
    timestamp: f64,
    invisible: bool,
    bytes: Vec<u8>,
}

pub struct WebmVideo {
    metadata: VideoMetadata,
    codec: DecoderCodec,
    packets: Vec<Packet>,
    decoder: Decoder,
    next_packet: usize,
    frame: Option<Arc<DecodedVideoFrame>>,
    retained_packet_bytes: usize,
    largest_padded_frame_pixels: usize,
}

impl WebmVideo {
    pub fn open(bytes: &[u8]) -> Result<Self, VideoError> {
        Self::open_with_control(bytes, &mut UnmanagedControl)
    }

    pub fn open_with_control(
        bytes: &[u8],
        control: &mut impl VideoDecodeControl,
    ) -> Result<Self, VideoError> {
        if control.is_cancelled() {
            return Err(VideoError::Cancelled);
        }
        if bytes.len() > MAX_ENCODED_VIDEO_BYTES {
            return Err(VideoError::Limit);
        }
        let mut container = MatroskaFile::open(Cursor::new(bytes))?;
        let track = container
            .tracks()
            .iter()
            .find(|track| track.track_type() == TrackType::Video && track.flag_enabled())
            .ok_or(VideoError::Unsupported)?;
        if track
            .content_encodings()
            .is_some_and(|encodings| !encodings.is_empty())
        {
            return Err(VideoError::Unsupported);
        }
        let codec = match track.codec_id() {
            "V_VP8" => DecoderCodec::Vp8,
            "V_VP9" => DecoderCodec::Vp9,
            _ => return Err(VideoError::Unsupported),
        };
        let track_number = track.track_number().get();
        let video = track.video().ok_or(VideoError::Unsupported)?;
        if video.alpha_mode().is_some_and(|mode| mode != 0) {
            return Err(VideoError::Unsupported);
        }
        let width = u32::try_from(video.pixel_width().get()).map_err(|_| VideoError::Limit)?;
        let height = u32::try_from(video.pixel_height().get()).map_err(|_| VideoError::Limit)?;
        checked_pixels(width, height)?;
        let largest_padded_frame_pixels = padded_frame_pixels(width, height)?;
        let frame_reserve = largest_padded_frame_pixels.saturating_mul(24);
        control.admit_retained_bytes(frame_reserve)?;
        let color = video.colour();
        let color_space = VideoColorSpace {
            matrix: color
                .and_then(|color| color.matrix_coefficients())
                .unwrap_or(MatrixCoefficients::Smpte170),
            primaries: color
                .and_then(|color| color.primaries())
                .unwrap_or(Primaries::Smpte170),
            transfer: color
                .and_then(|color| color.transfer_characteristics())
                .unwrap_or(TransferCharacteristics::Bt709),
            full_range: match color.and_then(|color| color.range()) {
                Some(Range::Full) => Some(true),
                Some(Range::Broadcast) => Some(false),
                _ => None,
            },
            profile_known: color.is_some_and(|color| {
                color
                    .primaries()
                    .is_some_and(|value| value != Primaries::Unknown)
                    || color
                        .transfer_characteristics()
                        .is_some_and(|value| value != TransferCharacteristics::Unknown)
            }),
        };
        let scale = container.info().timestamp_scale().get() as f64 / 1_000_000_000.0;
        let duration = container
            .info()
            .duration()
            .map(|duration| duration * scale)
            .filter(|duration| duration.is_finite() && *duration >= 0.0);
        let mut packets = Vec::new();
        let mut frame = Frame::default();
        let mut retained = 0usize;
        while container.next_frame(&mut frame)? {
            if control.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            if frame.track != track_number {
                continue;
            }
            retained = retained
                .checked_add(frame.data.len())
                .ok_or(VideoError::Limit)?;
            if retained > MAX_ENCODED_VIDEO_BYTES || packets.len() >= MAX_VIDEO_PACKETS {
                return Err(VideoError::Limit);
            }
            packets.push(Packet {
                timestamp: frame.timestamp as f64 * scale,
                invisible: frame.is_invisible
                    || (codec == DecoderCodec::Vp8
                        && frame.data.first().is_some_and(|byte| byte & 0x10 == 0)),
                bytes: std::mem::take(&mut frame.data),
            });
            let packet_storage = packets
                .capacity()
                .saturating_mul(std::mem::size_of::<Packet>());
            control.admit_retained_bytes(
                frame_reserve
                    .saturating_add(retained)
                    .saturating_add(packet_storage),
            )?;
        }
        if packets.is_empty() {
            return Err(VideoError::Empty);
        }
        retained = retained
            .checked_add(
                packets
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Packet>()),
            )
            .ok_or(VideoError::Limit)?;
        control.admit_retained_bytes(retained.saturating_add(frame_reserve))?;
        if control.is_cancelled() {
            return Err(VideoError::Cancelled);
        }
        let mut resource = Self {
            metadata: VideoMetadata {
                width,
                height,
                duration,
                color_space,
            },
            codec,
            packets,
            decoder: new_decoder(codec, largest_padded_frame_pixels)?,
            next_packet: 0,
            frame: None,
            retained_packet_bytes: retained,
            largest_padded_frame_pixels,
        };
        resource.frame_at_with_control(0.0, control)?;
        Ok(resource)
    }

    pub fn metadata(&self) -> VideoMetadata {
        self.metadata
    }

    /// Compressed packets plus the output and codec reference-frame allowance.
    pub fn retained_bytes(&self) -> usize {
        self.retained_packet_bytes
            .saturating_add(self.largest_padded_frame_pixels.saturating_mul(24))
    }

    pub fn frame_at(&mut self, seconds: f64) -> Result<Arc<DecodedVideoFrame>, VideoError> {
        self.frame_at_with_control(seconds, &mut UnmanagedControl)
    }

    pub fn current_frame(&self) -> Option<Arc<DecodedVideoFrame>> {
        self.frame.clone()
    }

    pub fn frame_at_with_control(
        &mut self,
        seconds: f64,
        control: &mut impl VideoDecodeControl,
    ) -> Result<Arc<DecodedVideoFrame>, VideoError> {
        if control.is_cancelled() {
            return Err(VideoError::Cancelled);
        }
        control.admit_retained_bytes(self.retained_bytes())?;
        let seconds = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        if self
            .frame
            .as_ref()
            .is_some_and(|frame| frame.timestamp > seconds)
        {
            self.decoder = new_decoder(self.codec, self.largest_padded_frame_pixels)?;
            self.next_packet = 0;
            self.frame = None;
        }
        while let Some(packet) = self.packets.get(self.next_packet) {
            if control.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            if self.frame.is_some() && !packet.invisible && packet.timestamp > seconds {
                break;
            }
            // Ask the codec to inspect independently decodable headers before
            // allocating native storage. VP9 additionally uses bounded buffers.
            if let Some((width, height)) =
                Decoder::peek_stream_dimensions(self.codec, &packet.bytes)
            {
                checked_pixels(width, height)?;
                self.largest_padded_frame_pixels = self
                    .largest_padded_frame_pixels
                    .max(padded_frame_pixels(width, height)?);
                control.admit_retained_bytes(self.retained_bytes())?;
                self.decoder
                    .increase_max_frame_bytes(self.largest_padded_frame_pixels.saturating_mul(4));
            }
            if control.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            self.decoder.decode(&packet.bytes)?;
            while let Some(frame) = self.decoder.next_frame()? {
                if !packet.invisible {
                    let frame = copy_frame(&frame, packet.timestamp, self.metadata.color_space)?;
                    self.largest_padded_frame_pixels = self
                        .largest_padded_frame_pixels
                        .max(padded_frame_pixels(frame.width, frame.height)?);
                    control.admit_retained_bytes(self.retained_bytes())?;
                    self.frame = Some(Arc::new(frame));
                }
            }
            self.next_packet += 1;
        }
        self.frame.clone().ok_or(VideoError::Empty)
    }
}

fn checked_pixels(width: u32, height: u32) -> Result<usize, VideoError> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(VideoError::Limit)?;
    if width == 0 || height == 0 || pixels > MAX_VIDEO_FRAME_PIXELS {
        return Err(VideoError::Limit);
    }
    Ok(pixels)
}

fn padded_frame_pixels(width: u32, height: u32) -> Result<usize, VideoError> {
    checked_pixels(width, height)?;
    let width = (width as usize)
        .checked_add(128)
        .ok_or(VideoError::Limit)?
        .div_ceil(64)
        * 64;
    let height = (height as usize)
        .checked_add(128)
        .ok_or(VideoError::Limit)?
        .div_ceil(64)
        * 64;
    width.checked_mul(height).ok_or(VideoError::Limit)
}

fn new_decoder(codec: DecoderCodec, padded_pixels: usize) -> Result<Decoder, VideoError> {
    Ok(Decoder::new_with_max_frame_bytes(
        DecoderConfig::new(codec),
        padded_pixels.saturating_mul(4),
    )?)
}

fn copy_frame(
    frame: &shiguredo_libvpx::DecodedFrame<'_>,
    timestamp: f64,
    mut color_space: VideoColorSpace,
) -> Result<DecodedVideoFrame, VideoError> {
    if frame.is_high_depth() {
        return Err(VideoError::Unsupported);
    }
    let width = u32::try_from(frame.width()).map_err(|_| VideoError::Limit)?;
    let height = u32::try_from(frame.height()).map_err(|_| VideoError::Limit)?;
    let pixels = checked_pixels(width, height)?;
    let cw = width.div_ceil(2) as usize;
    let ch = height.div_ceil(2) as usize;
    let size = pixels.checked_add(cw * ch * 2).ok_or(VideoError::Limit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| VideoError::Limit)?;
    for (plane, stride, width, height) in [
        (
            frame.y_plane(),
            frame.y_stride(),
            width as usize,
            height as usize,
        ),
        (frame.u_plane(), frame.u_stride(), cw, ch),
        (frame.v_plane(), frame.v_stride(), cw, ch),
    ] {
        for row in plane.chunks(stride).take(height) {
            bytes.extend_from_slice(row.get(..width).ok_or(VideoError::Unsupported)?);
        }
    }
    if bytes.len() != size {
        return Err(VideoError::Unsupported);
    }
    color_space.full_range = color_space.full_range.or(Some(frame.full_range()));
    if !color_space.profile_known {
        match frame.color_space() {
            1 | 3 => {
                color_space.matrix = MatrixCoefficients::Smpte170;
                color_space.primaries = Primaries::Smpte170;
                color_space.transfer = TransferCharacteristics::Smpte170;
                color_space.profile_known = true;
            }
            2 => {
                color_space.matrix = MatrixCoefficients::Bt709;
                color_space.primaries = Primaries::Bt709;
                color_space.transfer = TransferCharacteristics::Bt709;
                color_space.profile_known = true;
            }
            5 => {
                color_space.matrix = MatrixCoefficients::Bt2020Ncl;
                color_space.primaries = Primaries::Bt2020;
                color_space.transfer = TransferCharacteristics::Bt709;
                color_space.profile_known = true;
            }
            _ => {}
        }
    }
    Ok(DecodedVideoFrame {
        width,
        height,
        timestamp,
        color_space,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webm_frame_source_decodes_owned_planes_and_reuses_the_current_frame() {
        let mut source = WebmVideo::open(include_bytes!("../tests/fixtures/pattern.webm")).unwrap();
        let metadata = source.metadata();
        assert_eq!((metadata.width, metadata.height), (20, 20));
        let first = source.frame_at(0.0).unwrap();
        assert_eq!(
            (first.width, first.height, first.bytes.len()),
            (20, 20, 600)
        );
        assert_ne!(first.bytes[5 * 20 + 5], first.bytes[5 * 20 + 15]);
        assert!(Arc::ptr_eq(&first, &source.frame_at(0.0).unwrap()));
    }

    #[test]
    fn seeking_replays_codec_references_and_preserves_old_snapshots() {
        let mut source = WebmVideo::open(include_bytes!("../tests/fixtures/movie_5.webm")).unwrap();
        let first = source.frame_at(0.0).unwrap();
        let bytes = first.bytes.clone();
        let later = source.frame_at(2.5).unwrap();
        assert!(later.timestamp <= 2.5 && later.timestamp > 2.0);
        assert_ne!(first.bytes, later.bytes);
        let rewound = source.frame_at(0.0).unwrap();
        assert_eq!(rewound.bytes, bytes);
        assert_eq!(first.bytes, bytes);
    }

    #[test]
    fn truncated_or_oversized_container_elements_are_errors() {
        let bytes = include_bytes!("../tests/fixtures/pattern.webm");
        for size in [0, 1, 4, 20, 40] {
            assert!(WebmVideo::open(&bytes[..size]).is_err());
        }
        assert!(WebmVideo::open(&[0x1a, 0x45, 0xdf, 0xa3, 0x01, 0, 0, 0, 0, 0, 0, 1]).is_err());
    }

    #[test]
    fn vp9_display_frames_follow_container_timestamps_and_remain_owned() {
        let mut source =
            WebmVideo::open(include_bytes!("../tests/fixtures/three-frames-vp9.webm")).unwrap();
        assert_eq!(
            (source.metadata().width, source.metadata().height),
            (16, 16)
        );
        let first = source.frame_at(0.0).unwrap();
        for (time, luma) in [(0.0, 16), (1.0, 128), (2.0, 235)] {
            let frame = source.frame_at(time).unwrap();
            assert_eq!(frame.timestamp, time);
            assert!(frame.bytes[..256].iter().all(|sample| *sample == luma));
            assert!(frame.bytes[256..].iter().all(|sample| *sample == 128));
        }
        assert!(first.bytes[..256].iter().all(|sample| *sample == 16));
        assert_eq!(source.frame_at(0.0).unwrap().bytes, first.bytes);
    }

    #[test]
    fn vp9_refuses_native_frame_allocations_over_the_configured_limit() {
        let mut container = MatroskaFile::open(Cursor::new(include_bytes!(
            "../tests/fixtures/three-frames-vp9.webm"
        )))
        .unwrap();
        let mut packet = Frame::default();
        assert!(container.next_frame(&mut packet).unwrap());
        assert_eq!(
            Decoder::peek_stream_dimensions(DecoderCodec::Vp9, &packet.data),
            Some((16, 16))
        );
        let mut decoder =
            Decoder::new_with_max_frame_bytes(DecoderConfig::new(DecoderCodec::Vp9), 1).unwrap();
        assert!(decoder.decode(&packet.data).is_err());
        assert!(decoder.next_frame().unwrap().is_none());
    }
}
