use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use moli_image::RgbaImage;
use parking_lot::{Condvar, Mutex};
use tokio::sync::watch;

use crate::{VideoWriter, VideoWriterStats, writer::PreparedOutput};

const HEARTBEAT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub enum RecorderStatus {
    Starting,
    Recording { width: u32, height: u32 },
    Finished(VideoWriterStats),
    Failed(Arc<str>),
}

impl RecorderStatus {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished(_) | Self::Failed(_))
    }
}

struct Frame {
    image: RgbaImage,
    captured_at: Instant,
}

/// Maps capture-clock instants onto a continuous timeline without pauses.
struct RecordingTimeline {
    origin: Instant,
    segment_started_at: Instant,
    elapsed: Duration,
    paused: bool,
}

impl Default for RecordingTimeline {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            origin: now,
            segment_started_at: now,
            elapsed: Duration::ZERO,
            paused: false,
        }
    }
}

impl RecordingTimeline {
    fn timestamp(&self, now: Instant) -> Instant {
        self.origin
            + self.elapsed
            + if self.paused {
                Duration::ZERO
            } else {
                now.saturating_duration_since(self.segment_started_at)
            }
    }

    fn pause(&mut self, now: Instant) {
        if !self.paused {
            self.elapsed += now.saturating_duration_since(self.segment_started_at);
            self.paused = true;
        }
    }

    fn resume(&mut self, now: Instant) {
        if self.paused {
            self.segment_started_at = now;
            self.paused = false;
        }
    }
}

#[derive(Default)]
struct Queue {
    latest: Option<Frame>,
    stopped_at: Option<Instant>,
    replaced_frames: u64,
    failure: Option<Arc<str>>,
    timeline: RecordingTimeline,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    status: watch::Sender<RecorderStatus>,
}

/// An inexpensive producer endpoint. It never performs encoding or file I/O.
#[derive(Clone)]
pub struct RecorderHandle(Arc<Shared>);

impl RecorderHandle {
    pub fn needs_frame(&self) -> bool {
        let queue = self.0.queue.lock();
        queue.failure.is_none()
            && queue.stopped_at.is_none()
            && !queue.timeline.paused
            && queue.latest.is_none()
            && !self.0.status.borrow().is_terminal()
    }

    /// Accept the newest raw frame, replacing an unencoded frame if necessary.
    /// False means recording already stopped; no visual-state token may advance.
    pub fn offer(&self, image: RgbaImage, captured_at: Instant) -> bool {
        let mut queue = self.0.queue.lock();
        if queue.failure.is_some()
            || queue.stopped_at.is_some()
            || queue.timeline.paused
            || captured_at < queue.timeline.segment_started_at
            || self.0.status.borrow().is_terminal()
        {
            return false;
        }
        let captured_at = queue.timeline.timestamp(captured_at);
        let previous = queue.latest.replace(Frame { image, captured_at });
        if previous.is_some() {
            queue.replaced_frames += 1;
        }
        drop(queue);
        self.0.wake.notify_one();
        true
    }

    pub fn fail(&self, error: String) {
        let mut queue = self.0.queue.lock();
        queue.failure.get_or_insert(error.into());
        self.0.wake.notify_one();
    }

    pub fn stop(&self, stopped_at: Instant) {
        let mut queue = self.0.queue.lock();
        let stopped_at = queue.timeline.timestamp(stopped_at);
        queue.stopped_at.get_or_insert(stopped_at);
        self.0.wake.notify_one();
    }

    pub fn pause(&self, paused_at: Instant) {
        let mut queue = self.0.queue.lock();
        queue.timeline.pause(paused_at);
        self.0.wake.notify_one();
    }

    pub fn resume(&self, resumed_at: Instant) {
        let mut queue = self.0.queue.lock();
        queue.timeline.resume(resumed_at);
        self.0.wake.notify_one();
    }

    pub fn status(&self) -> RecorderStatus {
        self.0.status.borrow().clone()
    }
}

/// Owns one dedicated CPU encoder/muxer worker and its explicit completion.
/// Drop requests a stop; callers must await finish to report a saved video.
pub struct VideoRecorder {
    handle: RecorderHandle,
    status: watch::Receiver<RecorderStatus>,
    worker: Option<thread::JoinHandle<()>>,
}

impl VideoRecorder {
    pub fn spawn(path: &Path) -> Result<Self> {
        let output = PreparedOutput::create(path)?;
        let (status_tx, status) = watch::channel(RecorderStatus::Starting);
        let handle = RecorderHandle(Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            status: status_tx,
        }));
        let worker_state = handle.0.clone();
        let worker = thread::Builder::new()
            .name("moli-video".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    encode_frames(&worker_state, output)
                }));
                let status = match result {
                    Ok(Ok(stats)) => RecorderStatus::Finished(stats),
                    Ok(Err(error)) => RecorderStatus::Failed(format!("{error:#}").into()),
                    Err(_) => RecorderStatus::Failed("video worker panicked".into()),
                };
                worker_state.status.send_replace(status);
            })
            .context("could not start CPU video encoder thread")?;
        Ok(Self {
            handle,
            status,
            worker: Some(worker),
        })
    }

    pub fn handle(&self) -> RecorderHandle {
        self.handle.clone()
    }

    pub async fn initialized(&mut self) -> Result<()> {
        loop {
            let status = self.status.borrow_and_update().clone();
            match status {
                RecorderStatus::Starting => {}
                RecorderStatus::Recording { .. } | RecorderStatus::Finished(_) => return Ok(()),
                RecorderStatus::Failed(error) => bail!("screencast video failed: {error}"),
            }
            self.status
                .changed()
                .await
                .context("video worker status closed")?;
        }
    }

    /// A normal main-Page close finishes recording without closing the server.
    pub async fn failure(&mut self) -> anyhow::Error {
        loop {
            let status = self.status.borrow_and_update().clone();
            match status {
                RecorderStatus::Failed(error) => {
                    return anyhow!("screencast video failed: {error}");
                }
                RecorderStatus::Finished(_) => return std::future::pending().await,
                _ => {}
            }
            if self.status.changed().await.is_err() {
                return anyhow!("video worker status closed");
            }
        }
    }

    pub fn stop(&self, stopped_at: Instant) {
        self.handle.stop(stopped_at);
    }

    pub async fn finish(&mut self) -> Result<VideoWriterStats> {
        self.stop(Instant::now());
        let result = loop {
            let status = self.status.borrow_and_update().clone();
            match status {
                RecorderStatus::Finished(stats) => break Ok(stats),
                RecorderStatus::Failed(error) => {
                    break Err(anyhow!("screencast video failed: {error}"));
                }
                _ => {}
            }
            self.status
                .changed()
                .await
                .context("video worker status closed")?;
        };
        // The terminal status is sent only after writer/encoder resources have
        // been dropped. Join after is_finished so polling never blocks the IO
        // runtime, even if this task wins the race with the thread's epilogue.
        if let Some(worker) = self.worker.take() {
            while !worker.is_finished() {
                tokio::task::yield_now().await;
            }
            worker
                .join()
                .map_err(|_| anyhow!("video worker panicked"))?;
        }
        result
    }
}

impl Drop for VideoRecorder {
    fn drop(&mut self) {
        self.stop(Instant::now());
    }
}

fn encode_frames(shared: &Shared, output: PreparedOutput) -> Result<VideoWriterStats> {
    let mut output = Some(output);
    let mut writer: Option<VideoWriter> = None;
    let mut next_heartbeat = Instant::now() + HEARTBEAT;
    loop {
        let (frame, stopped_at, failure) = {
            let mut queue = shared.queue.lock();
            while queue.latest.is_none() && queue.stopped_at.is_none() && queue.failure.is_none() {
                if writer.is_some() && !queue.timeline.paused {
                    if shared
                        .wake
                        .wait_until(&mut queue, next_heartbeat)
                        .timed_out()
                    {
                        break;
                    }
                } else {
                    shared.wake.wait(&mut queue);
                }
            }
            (queue.latest.take(), queue.stopped_at, queue.failure.clone())
        };
        if let Some(error) = failure {
            bail!("{error}");
        }
        if let Some(frame) = frame {
            frame.image.validate()?;
            anyhow::ensure!(
                frame.image.width > 0 && frame.image.height > 0,
                "empty screencast frame"
            );
            let first = writer.is_none();
            if first {
                let (width, height) =
                    fit_dimensions(frame.image.width, frame.image.height, 1920, 1080);
                writer = Some(VideoWriter::from_output(
                    output.take().context("video output already initialized")?,
                    width,
                    height,
                    frame.captured_at,
                )?);
            }
            let active = writer.as_mut().context("video writer initialized above")?;
            let (width, height) = active.canvas_size();
            let image = fit_frame(frame.image, width, height)?;
            active.push_frame(&image, frame.captured_at)?;
            if first {
                shared
                    .status
                    .send_replace(RecorderStatus::Recording { width, height });
            }
            next_heartbeat = Instant::now() + HEARTBEAT;
        }
        if let Some(stopped_at) = stopped_at {
            let mut stats = if let Some(writer) = writer {
                writer.finish(stopped_at)?
            } else {
                output.take().context("unused video output")?.discard()?;
                VideoWriterStats::default()
            };
            stats.replaced_frames = shared.queue.lock().replaced_frames;
            return Ok(stats);
        }
        let now = Instant::now();
        if now >= next_heartbeat {
            let timestamp = {
                let queue = shared.queue.lock();
                (!queue.timeline.paused).then(|| queue.timeline.timestamp(now))
            };
            if let (Some(writer), Some(timestamp)) = (writer.as_mut(), timestamp) {
                writer.heartbeat(timestamp)?;
            }
            next_heartbeat = now + HEARTBEAT;
        }
    }
}

fn fit_dimensions(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    let scale = (f64::from(max_width) / f64::from(width))
        .min(f64::from(max_height) / f64::from(height))
        .min(1.0);
    (
        (f64::from(width) * scale).floor().max(1.0) as u32,
        (f64::from(height) * scale).floor().max(1.0) as u32,
    )
}

fn fit_frame(mut frame: RgbaImage, max_width: u32, max_height: u32) -> Result<RgbaImage> {
    if frame.width <= max_width && frame.height <= max_height {
        return Ok(frame);
    }
    // Composite before filtering so transparent pixels do not produce halos.
    for pixel in frame.rgba.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
            }
            pixel[3] = 255;
        }
    }
    let (width, height) = fit_dimensions(frame.width, frame.height, max_width, max_height);
    let input = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
        .context("invalid screencast raster")?;
    let fitted =
        image::imageops::resize(&input, width, height, image::imageops::FilterType::Triangle);
    Ok(RgbaImage::try_new(width, height, fitted.into_raw())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn producer_retains_only_the_newest_unencoded_frame() -> Result<()> {
        let (status, _) = watch::channel(RecorderStatus::Starting);
        let handle = RecorderHandle(Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            status,
        }));
        let started = Instant::now();
        for tick in 0..10 {
            assert!(handle.offer(
                RgbaImage::try_new(2, 2, vec![tick; 16])?,
                started + Duration::from_millis(u64::from(tick))
            ));
        }
        let queue = handle.0.queue.lock();
        assert_eq!(queue.replaced_frames, 9);
        assert_eq!(queue.latest.as_ref().unwrap().image.rgba, vec![9; 16]);
        drop(queue);
        handle.stop(started + Duration::from_secs(1));
        assert!(!handle.needs_frame());
        assert!(!handle.offer(RgbaImage::try_new(2, 2, vec![0; 16])?, started));
        Ok(())
    }

    #[tokio::test]
    async fn worker_finishes_without_a_client_or_further_frames() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("video.mp4");
        let mut recorder = VideoRecorder::spawn(&path)?;
        let handle = recorder.handle();
        assert!(handle.offer(RgbaImage::try_new(3, 3, vec![255; 36])?, Instant::now()));
        tokio::time::timeout(Duration::from_secs(5), recorder.initialized()).await??;
        assert!(!path.exists());
        let stats = tokio::time::timeout(Duration::from_secs(5), recorder.finish()).await??;
        assert_eq!(stats.encoded_frames, 1);
        assert!(path.exists());
        assert!(!handle.offer(RgbaImage::try_new(2, 2, vec![0; 16])?, Instant::now()));
        assert_eq!(recorder.finish().await?.encoded_frames, 1);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_frame_is_a_persistent_worker_error() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("video.mp4");
        let mut recorder = VideoRecorder::spawn(&path)?;
        recorder.handle().offer(
            RgbaImage {
                width: 2,
                height: 2,
                rgba: Vec::new(),
            },
            Instant::now(),
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), recorder.initialized())
                .await?
                .is_err()
        );
        assert!(recorder.finish().await.is_err());
        assert!(!path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn unused_sink_finishes_without_publishing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("empty.mp4");
        let mut recorder = VideoRecorder::spawn(&path)?;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), recorder.finish())
                .await??
                .encoded_frames,
            0
        );
        assert!(!path.exists());
        assert!(dir.path().read_dir()?.next().is_none());
        Ok(())
    }

    #[tokio::test]
    async fn repeated_pauses_are_cut_from_one_video_timeline() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("resumed.mp4");
        let mut recorder = VideoRecorder::spawn(&path)?;
        let handle = recorder.handle();
        let started = Instant::now();
        assert!(handle.offer(RgbaImage::try_new(2, 2, vec![255; 16])?, started));
        tokio::time::timeout(Duration::from_secs(5), recorder.initialized()).await??;
        handle.pause(started + Duration::from_millis(350));
        handle.pause(started + Duration::from_secs(1));
        assert!(!handle.needs_frame());
        assert!(!handle.offer(
            RgbaImage::try_new(2, 2, vec![0; 16])?,
            started + Duration::from_secs(5)
        ));
        handle.resume(started + Duration::from_secs(10));
        handle.resume(started + Duration::from_secs(10));
        assert!(handle.offer(
            RgbaImage::try_new(2, 2, [255, 0, 0, 255].repeat(4))?,
            started + Duration::from_millis(10_100)
        ));
        handle.pause(started + Duration::from_millis(10_200));
        handle.stop(started + Duration::from_secs(30));
        let stats = recorder.finish().await?;
        assert_eq!(stats.encoded_frames, 2);
        assert_eq!(stats.duration_ticks, 49_500);
        let reader =
            mp4::Mp4Reader::read_header(std::fs::File::open(&path)?, path.metadata()?.len())?;
        assert_eq!(reader.tracks()[&1].duration(), Duration::from_millis(550));
        Ok(())
    }

    #[tokio::test]
    async fn large_shared_screencast_is_fitted_to_the_video_canvas() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("fitted.mp4");
        let mut recorder = VideoRecorder::spawn(&path)?;
        recorder.handle().offer(
            RgbaImage::try_new(1921, 1, [0, 0, 255, 255].repeat(1921))?,
            Instant::now(),
        );
        tokio::time::timeout(Duration::from_secs(5), recorder.initialized()).await??;
        recorder.finish().await?;
        let reader =
            mp4::Mp4Reader::read_header(std::fs::File::open(&path)?, path.metadata()?.len())?;
        assert_eq!(
            (reader.tracks()[&1].width(), reader.tracks()[&1].height()),
            (1920, 2)
        );
        Ok(())
    }
}
