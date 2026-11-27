use anyhow::{Result, anyhow, ensure};
use bytes::Bytes;
use parking_lot::Mutex;
use rsmpeg::avcodec::AVPacket;
use rsmpeg::avformat::{
    AVFormatContextInput, AVIOContextContainer, AVIOContextCustom, AVInputFormat,
};
use rsmpeg::avutil::{AVDictionary, AVMem};
use rsmpeg::ffi;
use std::sync::Arc;

use super::h264::{Decoded, H264Decoder, MICROSECONDS, VideoFrame};

#[derive(Clone, Copy, Debug)]
pub struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration: Option<f64>,
}

pub struct Mp4Decoder {
    input: Mp4Input,
    decoder: H264Decoder,
    start: i64,
    next_timestamp: f64,
    draining: bool,
}

impl Mp4Decoder {
    pub fn open(bytes: Bytes) -> Result<Self> {
        let input = Mp4Input::open(vec![bytes])?;
        let stream = &input.format.streams()[input.video_stream];
        let decoder = H264Decoder::new(&stream.codecpar(), false)?;
        let start = if stream.start_time == ffi::AV_NOPTS_VALUE {
            0
        } else {
            stream.start_time
        };
        let start = rescale(start, input.time_base, MICROSECONDS);
        Ok(Self {
            input,
            decoder,
            start,
            next_timestamp: 0.0,
            draining: false,
        })
    }

    pub fn metadata(&self) -> VideoMetadata {
        self.input.metadata
    }

    /// Seek to a keyframe at or before the requested presentation time.
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        ensure!(seconds.is_finite() && seconds >= 0.0, "invalid video time");
        let timestamp = ((seconds * 1_000_000.0) as i64).saturating_add(self.start);
        self.input.format.seek(
            self.input.video_stream as i32,
            rescale(timestamp, MICROSECONDS, self.input.time_base),
            ffi::AVSEEK_FLAG_BACKWARD as i32,
        )?;
        self.decoder.reset();
        self.draining = false;
        self.next_timestamp = seconds;
        Ok(())
    }

    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        loop {
            match self.decoder.receive()? {
                Decoded::Frame(frame) => {
                    return self
                        .decoder
                        .frame(frame, self.start, &mut self.next_timestamp)
                        .map(Some);
                }
                Decoded::NeedsPacket if !self.draining => {
                    let packet = self.input.read_video_packet()?;
                    self.draining = packet.is_none();
                    self.decoder.send(packet.as_ref())?;
                }
                Decoded::NeedsPacket | Decoded::EndOfStream => return Ok(None),
            }
        }
    }
}

pub(super) struct Mp4Input {
    format: AVFormatContextInput,
    video_stream: usize,
    time_base: ffi::AVRational,
    pub(super) metadata: VideoMetadata,
}

impl Mp4Input {
    pub(super) fn open(parts: Vec<Bytes>) -> Result<Self> {
        let format = open_mp4(parts)?;
        let (video_stream, stream) = format
            .streams()
            .iter()
            .enumerate()
            .find(|(_, stream)| {
                let parameters = stream.codecpar();
                parameters.codec_type == ffi::AVMEDIA_TYPE_VIDEO
                    && parameters.codec_id == ffi::AV_CODEC_ID_H264
                    && stream.disposition & ffi::AV_DISPOSITION_ATTACHED_PIC as i32 == 0
            })
            .ok_or_else(|| anyhow!("no H.264 video track in MP4"))?;
        let parameters = stream.codecpar();
        let time_base = stream.time_base;
        ensure!(
            time_base.num > 0 && time_base.den > 0,
            "invalid video time base"
        );
        let duration = if stream.duration > 0 {
            Some(as_seconds(rescale(
                stream.duration,
                time_base,
                MICROSECONDS,
            )))
        } else {
            (format.duration > 0).then(|| as_seconds(format.duration))
        };
        let metadata = VideoMetadata {
            width: u32::try_from(parameters.width)?,
            height: u32::try_from(parameters.height)?,
            duration,
        };
        drop(parameters);
        Ok(Self {
            format,
            video_stream,
            time_base,
            metadata,
        })
    }

    pub(super) fn h264_decoder(&self) -> Result<H264Decoder> {
        let stream = &self.format.streams()[self.video_stream];
        H264Decoder::new(&stream.codecpar(), true)
    }

    pub(super) fn read_video_packet(&mut self) -> Result<Option<AVPacket>> {
        read_track_packet(&mut self.format, self.video_stream, self.time_base)
    }
}

pub(super) fn open_mp4(parts: Vec<Bytes>) -> Result<AVFormatContextInput> {
    let mov =
        AVInputFormat::find(c"mov").ok_or_else(|| anyhow!("FFmpeg MP4 demuxer is unavailable"))?;
    let io = open_memory(parts);
    // Never let an externally supplied data reference open another input.
    let mut options = Some(AVDictionary::new(c"enable_drefs", c"0", 0));
    Ok(AVFormatContextInput::builder()
        .format(&mov)
        .io_context(AVIOContextContainer::Custom(io))
        .options(&mut options)
        .probe_stream_info(false)
        .open()?)
}

pub(super) fn read_track_packet(
    format: &mut AVFormatContextInput,
    stream: usize,
    time_base: ffi::AVRational,
) -> Result<Option<AVPacket>> {
    while let Some(mut packet) = format.read_packet()? {
        if packet.stream_index == stream as i32 {
            packet.rescale_ts(time_base, MICROSECONDS);
            return Ok(Some(packet));
        }
    }
    Ok(None)
}

fn rescale(timestamp: i64, from: ffi::AVRational, to: ffi::AVRational) -> i64 {
    // The demuxer time base is checked once at open. MICROSECONDS is constant.
    unsafe { ffi::av_rescale_q(timestamp, from, to) }
}

pub(super) fn as_seconds(microseconds: i64) -> f64 {
    microseconds as f64 / 1_000_000.0
}

struct MemoryInput {
    parts: Vec<Bytes>,
    position: i64,
    len: i64,
}

impl MemoryInput {
    fn read(&mut self, out: &mut [u8]) -> i32 {
        let mut start = self.position as usize;
        let mut written = 0;
        for part in &self.parts {
            if start >= part.len() {
                start -= part.len();
                continue;
            }
            let n = (part.len() - start).min(out.len() - written);
            out[written..written + n].copy_from_slice(&part[start..start + n]);
            written += n;
            start = 0;
            if written == out.len() {
                break;
            }
        }
        self.position += written as i64;
        if written == 0 {
            ffi::AVERROR_EOF
        } else {
            written as i32
        }
    }

    fn seek(&mut self, offset: i64, whence: i32) -> i64 {
        let origin = match whence & !(ffi::AVSEEK_FORCE as i32) {
            0 => 0,
            1 => self.position,
            2 => self.len,
            size if size == ffi::AVSEEK_SIZE as i32 => return self.len,
            _ => return -i64::from(ffi::EINVAL),
        };
        if let Some(position) = origin
            .checked_add(offset)
            .filter(|&position| (0..=self.len).contains(&position))
        {
            self.position = position;
            position
        } else {
            -i64::from(ffi::EINVAL)
        }
    }
}

fn open_memory(parts: Vec<Bytes>) -> AVIOContextCustom {
    let len = parts.iter().map(|part| part.len() as i64).sum();
    let input = Arc::new(Mutex::new(MemoryInput {
        parts,
        position: 0,
        len,
    }));
    let reader = input.clone();
    AVIOContextCustom::alloc_context(
        AVMem::new(32 * 1024),
        false,
        Vec::new(),
        Some(Box::new(move |_, out| reader.lock().read(out))),
        None,
        Some(Box::new(move |_, offset, whence| {
            input.lock().seek(offset, whence)
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drains_and_seeks_h264_frames_in_presentation_order() {
        let bytes = Bytes::from_static(include_bytes!("../tests/fixtures/primary-colors.mp4"));
        let mut decoder = Mp4Decoder::open(bytes).unwrap();
        let metadata = decoder.metadata();
        assert_eq!((metadata.width, metadata.height), (48, 32));
        assert_eq!(metadata.duration, Some(2.0));

        for index in 0..8 {
            let frame = decoder.next_frame().unwrap().unwrap();
            assert_eq!(frame.timestamp, f64::from(index) / 4.0);
            assert_eq!(frame.duration, 0.25);
            let pixel = &frame.image.rgba[..4];
            let dominant = if index < 4 { 0 } else { 2 };
            assert!(pixel[dominant] > 240, "{pixel:?}");
            assert!(pixel[2 - dominant] < 12, "{pixel:?}");
            assert_eq!(pixel[3], 255);
        }
        assert!(decoder.next_frame().unwrap().is_none());

        decoder.seek(1.5).unwrap();
        let frame = decoder.next_frame().unwrap().unwrap();
        assert_eq!(frame.timestamp, 1.0);
        assert!(frame.image.rgba[2] > 240);
    }
}
