use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result, ensure};
use moli_image::RgbaImage;
use mp4::{AvcConfig, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
use tempfile::NamedTempFile;

use crate::encoder::{EncodedFrame, Encoder};

pub const TIMESCALE: u32 = 90_000;
const IDR_TICKS: u64 = 2 * TIMESCALE as u64;

#[derive(Clone, Debug, Default)]
pub struct VideoWriterStats {
    pub encoded_frames: u64,
    pub duration_ticks: u64,
    pub encoded_bytes: u64,
    pub replaced_frames: u64,
}

pub(crate) struct PreparedOutput {
    file: Option<NamedTempFile>,
    destination: PathBuf,
}

impl PreparedOutput {
    pub(crate) fn create(path: &Path) -> Result<Self> {
        ensure!(!path.as_os_str().is_empty(), "video path cannot be empty");
        ensure!(
            !path.try_exists()?,
            "video destination already exists: {}",
            path.display()
        );
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file = tempfile::Builder::new()
            .prefix(".moli-video-")
            .suffix(".part")
            .tempfile_in(parent)
            .with_context(|| format!("cannot create video in {}", parent.display()))?;
        Ok(Self {
            file: Some(file),
            destination: path.into(),
        })
    }

    fn publish(&mut self) -> Result<()> {
        let output = self.file.take().context("video output already published")?;
        if let Err(error) = output.persist_noclobber(&self.destination) {
            let source = error.error;
            let (_, path) = error.file.keep()?;
            return Err(source).with_context(|| {
                format!(
                    "cannot publish video to {}; recording retained at {}",
                    self.destination.display(),
                    path.display()
                )
            });
        }
        Ok(())
    }

    pub(crate) fn discard(mut self) -> Result<()> {
        if let Some(file) = self.file.take() {
            file.close()?;
        }
        Ok(())
    }
}

impl Drop for PreparedOutput {
    fn drop(&mut self) {
        if let Some(output) = self.file.take() {
            let _ = output.keep();
        }
    }
}

/// One fixed-size, CPU-encoded AVC track. Construct and use on one thread.
pub struct VideoWriter {
    encoder: Encoder,
    muxer: Option<Mp4Writer<BufWriter<File>>>,
    output: PreparedOutput,
    canvas: Vec<u8>,
    width: u32,
    height: u32,
    started: Instant,
    pending: Option<EncodedFrame>,
    last_pts: Option<u64>,
    last_idr: u64,
    stats: VideoWriterStats,
}

impl VideoWriter {
    pub fn new(path: &Path, width: u32, height: u32, started: Instant) -> Result<Self> {
        Self::from_output(PreparedOutput::create(path)?, width, height, started)
    }

    pub(crate) fn from_output(
        output: PreparedOutput,
        width: u32,
        height: u32,
        started: Instant,
    ) -> Result<Self> {
        ensure!(
            width > 0 && height > 0 && width <= 1920 && height <= 1080,
            "video canvas must fit 1920x1080"
        );
        let width = (width + 1) & !1;
        let height = (height + 1) & !1;
        let encoder = Encoder::new(width, height)?;
        let file = output
            .file
            .as_ref()
            .context("video output unavailable")?
            .reopen()?;
        let mut muxer = Mp4Writer::write_start(
            BufWriter::new(file),
            &Mp4Config {
                major_brand: "isom".parse()?,
                minor_version: 512,
                compatible_brands: vec![
                    "isom".parse()?,
                    "iso2".parse()?,
                    "avc1".parse()?,
                    "mp41".parse()?,
                ],
                timescale: TIMESCALE,
            },
        )?;
        let mut track = TrackConfig::from(AvcConfig {
            width: width as u16,
            height: height as u16,
            seq_param_set: encoder.sps.clone(),
            pic_param_set: encoder.pps.clone(),
        });
        track.timescale = TIMESCALE;
        muxer.add_track(&track)?;
        Ok(Self {
            encoder,
            muxer: Some(muxer),
            output,
            canvas: Vec::new(),
            width,
            height,
            started,
            pending: None,
            last_pts: None,
            last_idr: 0,
            stats: VideoWriterStats::default(),
        })
    }

    pub fn canvas_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn push_frame(&mut self, image: &RgbaImage, captured_at: Instant) -> Result<()> {
        image.validate()?;
        ensure!(
            image.width <= self.width && image.height <= self.height,
            "video frame exceeds fixed canvas"
        );
        let x = (self.width - image.width) / 2;
        let y = (self.height - image.height) / 2;
        // A reduction lets the compiler vectorize the alpha scan. Fully opaque
        // frames that already fit the canvas need no copy or compositing.
        let opaque = image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .fold(u8::MAX, |alpha, pixel| alpha & pixel[3])
            == u8::MAX;
        let pixels = if image.width == self.width && image.height == self.height && opaque {
            image.rgba.as_slice()
        } else {
            // Only padding or transparency needs a separate RGBA canvas.
            self.canvas
                .resize(self.width as usize * self.height as usize * 4, 255);
            if image.width != self.width || image.height != self.height {
                self.canvas.fill(255);
            }
            for row in 0..image.height as usize {
                let source = row * image.width as usize * 4;
                let dest = ((row + y as usize) * self.width as usize + x as usize) * 4;
                let source = &image.rgba[source..source + image.width as usize * 4];
                let dest = &mut self.canvas[dest..dest + image.width as usize * 4];
                if opaque {
                    dest.copy_from_slice(source);
                } else {
                    for (source, dest) in source
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(dest.as_chunks_mut::<4>().0)
                    {
                        let alpha = u32::from(source[3]);
                        for channel in 0..3 {
                            dest[channel] =
                                ((u32::from(source[channel]) * alpha + 255 * (255 - alpha) + 127)
                                    / 255) as u8;
                        }
                    }
                }
            }
            self.canvas.as_slice()
        };
        yuv::rgba_to_yuv420(
            &mut self.encoder.planes,
            pixels,
            self.width * 4,
            yuv::YuvRange::Limited,
            yuv::YuvStandardMatrix::Bt709,
            yuv::YuvConversionMode::Balanced,
        )?;
        self.encode_at(captured_at)
    }

    /// Extend a static picture without rerasterizing or converting it.
    pub fn heartbeat(&mut self, now: Instant) -> Result<()> {
        if self.last_pts.is_some() && self.ticks(now)?.saturating_sub(self.last_idr) >= IDR_TICKS {
            self.encode_at(now)?;
        }
        Ok(())
    }

    fn ticks(&self, time: Instant) -> Result<u64> {
        let elapsed = time
            .checked_duration_since(self.started)
            .context("video timestamp predates first capture")?;
        u64::try_from(elapsed.as_nanos() * u128::from(TIMESCALE) / 1_000_000_000)
            .context("video timestamp overflow")
    }

    fn encode_at(&mut self, time: Instant) -> Result<()> {
        let pts = self
            .ticks(time)?
            .max(self.last_pts.map_or(0, |v| v.saturating_add(1)));
        ensure!(
            self.last_pts.is_some() || pts == 0,
            "first video frame must match recording epoch"
        );
        let idr = self.last_pts.is_none() || pts.saturating_sub(self.last_idr) >= IDR_TICKS;
        if idr {
            self.last_idr = pts;
        }
        if let Some(frame) = self.encoder.encode(pts, idr)? {
            self.accept(frame)?;
        }
        self.last_pts = Some(pts);
        Ok(())
    }

    fn accept(&mut self, frame: EncodedFrame) -> Result<()> {
        if let Some(previous) = self.pending.take() {
            self.write_sample(previous, frame.pts)?;
        }
        self.stats.encoded_frames += 1;
        self.stats.encoded_bytes += frame.bytes.len() as u64;
        self.pending = Some(frame);
        Ok(())
    }

    fn write_sample(&mut self, frame: EncodedFrame, end: u64) -> Result<()> {
        let duration = u32::try_from(
            end.checked_sub(frame.pts)
                .filter(|d| *d > 0)
                .context("non-monotonic video timestamp")?,
        )
        .context("video sample duration exceeds MP4 range")?;
        self.muxer
            .as_mut()
            .context("video already finished")?
            .write_sample(
                1,
                &Mp4Sample {
                    start_time: frame.pts,
                    duration,
                    rendering_offset: 0,
                    is_sync: frame.keyframe,
                    bytes: frame.bytes,
                },
            )?;
        self.stats.duration_ticks = end;
        Ok(())
    }

    pub fn finish(mut self, stopped_at: Instant) -> Result<VideoWriterStats> {
        while let Some(frame) = self.encoder.drain()? {
            self.accept(frame)?;
        }
        let frame = self.pending.take().context("video has no encoded frames")?;
        let end = self.ticks(stopped_at)?.max(frame.pts.saturating_add(1));
        self.write_sample(frame, end)?;
        let mut muxer = self.muxer.take().context("video already finished")?;
        muxer.write_end()?;
        let mut file = muxer
            .into_writer()
            .into_inner()
            .context("flushing MP4 output")?;
        file.flush()?;
        drop(file);
        self.output.publish()?;
        Ok(self.stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn opaque_frames_avoid_scratch_and_alpha_padding_preserve_pixels() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let started = Instant::now();
        let mut writer = VideoWriter::new(&directory.path().join("pixels.mp4"), 4, 4, started)?;
        let colors = [
            (4, 4, [255, 0, 0, 255]),
            (4, 4, [255, 0, 0, 128]),
            (2, 2, [0, 0, 255, 255]),
            (4, 4, [0, 255, 0, 255]),
        ];
        for (index, (width, height, color)) in colors.into_iter().enumerate() {
            let image = RgbaImage::try_new(width, height, color.repeat((width * height) as usize))?;
            writer.push_frame(&image, started + Duration::from_millis(index as u64 * 100))?;
            if index == 0 {
                assert_eq!(
                    writer.canvas.capacity(),
                    0,
                    "opaque frames borrow their original raster"
                );
            }
            let rgba = match index {
                0 => [255, 0, 0, 255].repeat(16),
                1 => [255, 127, 127, 255].repeat(16),
                2 => {
                    let mut rgba = vec![255; 64];
                    for pixel in [5, 6, 9, 10] {
                        rgba[pixel * 4..pixel * 4 + 4].copy_from_slice(&[0, 0, 255, 255]);
                    }
                    rgba
                }
                3 => [0, 255, 0, 255].repeat(16),
                _ => unreachable!(),
            };
            let mut expected =
                yuv::YuvPlanarImageMut::alloc(4, 4, yuv::YuvChromaSubsampling::Yuv420);
            yuv::rgba_to_yuv420(
                &mut expected,
                &rgba,
                16,
                yuv::YuvRange::Limited,
                yuv::YuvStandardMatrix::Bt709,
                yuv::YuvConversionMode::Balanced,
            )?;
            assert_eq!(
                writer.encoder.planes.y_plane.borrow(),
                expected.y_plane.borrow()
            );
            assert_eq!(
                writer.encoder.planes.u_plane.borrow(),
                expected.u_plane.borrow()
            );
            assert_eq!(
                writer.encoder.planes.v_plane.borrow(),
                expected.v_plane.borrow()
            );
        }
        assert_eq!(
            writer
                .finish(started + Duration::from_millis(400))?
                .encoded_frames,
            4
        );
        Ok(())
    }

    #[test]
    fn vfr_static_tail_and_odd_canvas_are_preserved() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("video.mp4");
        let start = Instant::now();
        let mut writer = VideoWriter::new(&path, 63, 47, start)?;
        let image = RgbaImage::try_new(63, 47, [255, 0, 0, 255].repeat(63 * 47))?;
        writer.push_frame(&image, start)?;
        writer.push_frame(&image, start + Duration::from_millis(350))?;
        writer.heartbeat(start + Duration::from_secs(10))?;
        assert!(!path.exists());
        let stats = writer.finish(start + Duration::from_secs(12))?;
        assert_eq!(stats.duration_ticks, 12 * u64::from(TIMESCALE));
        let file = File::open(&path)?;
        let reader = mp4::Mp4Reader::read_header(file, path.metadata()?.len())?;
        let track = reader.tracks().get(&1).context("AVC track")?;
        assert_eq!((track.width(), track.height()), (64, 48));
        assert_eq!(track.timescale(), TIMESCALE);
        assert_eq!(track.duration(), Duration::from_secs(12));
        assert_eq!(track.sample_count(), 3);
        assert_eq!(track.media_type()?, mp4::MediaType::H264);
        Ok(())
    }

    #[test]
    fn destination_race_never_overwrites_another_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("video.mp4");
        let now = Instant::now();
        let mut writer = VideoWriter::new(&path, 2, 2, now)?;
        writer.push_frame(&RgbaImage::try_new(2, 2, vec![255; 16])?, now)?;
        std::fs::write(&path, b"another writer")?;
        assert!(writer.finish(now + Duration::from_secs(1)).is_err());
        assert_eq!(std::fs::read(path)?, b"another writer");
        Ok(())
    }

    #[test]
    fn recording_duration_uses_64_bit_mp4_headers() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("long.mp4");
        let start = Instant::now();
        let mut writer = VideoWriter::new(&path, 2, 2, start)?;
        writer.push_frame(&RgbaImage::try_new(2, 2, vec![255; 16])?, start)?;
        for hour in 1..=24 {
            writer.heartbeat(start + Duration::from_secs(hour * 3600))?;
        }
        let duration = Duration::from_secs(25 * 3600);
        writer.finish(start + duration)?;
        let reader = mp4::Mp4Reader::read_header(File::open(&path)?, path.metadata()?.len())?;
        assert_eq!(reader.duration(), duration);
        assert_eq!(reader.tracks()[&1].duration(), duration);
        Ok(())
    }
}
