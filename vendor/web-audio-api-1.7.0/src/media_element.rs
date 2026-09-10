use std::error::Error;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use creek::{ReadDiskStream, SeekMode, SymphoniaDecoder};
use crossbeam_channel::{Receiver, Sender};

use crate::{AtomicF64, AudioBuffer, RENDER_QUANTUM_SIZE};

/// Real time safe audio stream
pub(crate) struct RTSStream {
    stream: ReadDiskStream<SymphoniaDecoder>,
    number_of_channels: usize,
    current_time: Arc<AtomicF64>,
    receiver: Receiver<MediaElementAction>,
    loop_: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    playback_rate: Arc<AtomicF64>,
}

impl std::fmt::Debug for RTSStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RTSStream")
            .field("number_of_channels", &self.number_of_channels)
            .finish_non_exhaustive()
    }
}

/// Controller actions for a media element
pub(crate) enum MediaElementAction {
    /// Seek to the given timestamp
    Seek(f64),
    /// Enable/disable looping
    SetLoop(bool),
    /// Start or restart the stream
    Play,
    /// Pause the stream
    Pause,
    /// Update the playback rate
    SetPlaybackRate(f64),
}

/// Shim of the `<audio>` element which allows you to efficiently play and seek audio from disk
///
/// The documentation for [`MediaElementAudioSourceNode`](crate::node::MediaElementAudioSourceNode)
/// contains usage instructions.
pub struct MediaElement {
    stream: Option<RTSStream>,
    current_time: Arc<AtomicF64>,
    sender: Sender<MediaElementAction>,
    loop_: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    playback_rate: Arc<AtomicF64>,
}

impl std::fmt::Debug for MediaElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaElement")
            .field("stream", &self.stream)
            .field("current_time", &self.current_time())
            .field("loop", &self.loop_())
            .field("paused", &self.paused())
            .field("playback_rate", &self.playback_rate())
            .finish_non_exhaustive()
    }
}

impl MediaElement {
    /// Create a new instance for a given file path
    pub fn new<P: Into<PathBuf>>(file: P) -> Result<Self, Box<dyn Error>> {
        // Open a read stream.
        let mut read_disk_stream = ReadDiskStream::<SymphoniaDecoder>::new(
            file,               // Path to file.
            0,                  // The frame in the file to start reading from.
            Default::default(), // Use default read stream options.
        )?;
        let number_of_channels = read_disk_stream.info().num_channels as usize;

        // Cache the start of the file into cache with index `0`.
        let _ = read_disk_stream.cache(0, 0);

        // Tell the stream to seek to the beginning of file. This will also alert the stream to the existence
        // of the cache with index `0`.
        read_disk_stream.seek(0, SeekMode::default())?;

        // Wait until the buffer is filled before sending it to the process thread.
        read_disk_stream.block_until_ready()?;

        // Setup control/render thread message bus
        // Use a bounded channel for real-time safety. A maximum of 32 control messages (start,
        // seek, ..) will be handled per render quantum. The control thread will block when the
        // capacity is reached.
        let (sender, receiver) = crossbeam_channel::bounded(32);
        // Setup currentTime shared value
        let current_time = Arc::new(AtomicF64::new(0.));

        let loop_ = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(true));
        let playback_rate = Arc::new(AtomicF64::new(1.));

        let rts_stream = RTSStream {
            stream: read_disk_stream,
            number_of_channels,
            current_time: Arc::clone(&current_time),
            receiver,
            loop_: Arc::clone(&loop_),
            paused: Arc::clone(&paused),
            playback_rate: Arc::clone(&playback_rate),
        };

        Ok(Self {
            stream: Some(rts_stream),
            current_time,
            sender,
            loop_,
            paused,
            playback_rate,
        })
    }

    pub(crate) fn take_stream(&mut self) -> Option<RTSStream> {
        self.stream.take()
    }

    pub fn current_time(&self) -> f64 {
        self.current_time.load(Ordering::SeqCst)
    }

    pub fn set_current_time(&self, value: f64) {
        let _ = self.sender.send(MediaElementAction::Seek(value));
    }

    pub fn loop_(&self) -> bool {
        self.loop_.load(Ordering::SeqCst)
    }

    pub fn set_loop(&self, value: bool) {
        let _ = self.sender.send(MediaElementAction::SetLoop(value));
    }

    pub fn play(&self) {
        let _ = self.sender.send(MediaElementAction::Play);
    }

    pub fn pause(&self) {
        let _ = self.sender.send(MediaElementAction::Pause);
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn playback_rate(&self) -> f64 {
        self.playback_rate.load(Ordering::SeqCst)
    }

    pub fn set_playback_rate(&self, value: f64) {
        let _ = self.sender.send(MediaElementAction::SetPlaybackRate(value));
    }
}

impl Iterator for RTSStream {
    type Item = Result<AudioBuffer, Box<dyn Error + Send + Sync>>;

    fn next(&mut self) -> Option<Self::Item> {
        let sample_rate = self.stream.info().sample_rate.unwrap() as f32;

        if let Ok(msg) = self.receiver.try_recv() {
            use MediaElementAction::*;
            match msg {
                Seek(value) => {
                    self.current_time.store(value, Ordering::SeqCst);
                    let frame = (value * sample_rate as f64) as usize;
                    self.stream.seek(frame, SeekMode::default()).unwrap();
                }
                SetLoop(value) => {
                    self.loop_.store(value, Ordering::SeqCst);
                }
                Play => self.paused.store(false, Ordering::SeqCst),
                Pause => self.paused.store(true, Ordering::SeqCst),
                SetPlaybackRate(value) => self.playback_rate.store(value, Ordering::SeqCst),
            };
        }

        if self.paused.load(Ordering::SeqCst) {
            let silence = AudioBuffer::from(
                vec![vec![0.; RENDER_QUANTUM_SIZE]; self.number_of_channels],
                sample_rate,
            );
            return Some(Ok(silence));
        }

        let playback_rate = self.playback_rate.load(Ordering::SeqCst).abs();
        let _reverse = playback_rate < 0.; // TODO
        let samples = (RENDER_QUANTUM_SIZE as f64 * playback_rate) as usize;

        let next = match self.stream.read(samples) {
            Ok(data) => {
                let channels: Vec<_> = (0..data.num_channels())
                    .map(|i| data.read_channel(i).to_vec())
                    .collect();
                let buf = AudioBuffer::from(channels, sample_rate * playback_rate as f32);

                if self.loop_.load(Ordering::SeqCst) && data.reached_end_of_file() {
                    self.stream.seek(0, SeekMode::default()).unwrap();
                    self.current_time.store(0., Ordering::SeqCst);
                } else {
                    let current_time = self.current_time.load(Ordering::SeqCst);
                    self.current_time.store(
                        current_time + (RENDER_QUANTUM_SIZE as f64 / sample_rate as f64),
                        Ordering::SeqCst,
                    );
                }

                Ok(buf)
            }
            Err(e) => Err(Box::new(e) as _),
        };

        Some(next)
    }
}
