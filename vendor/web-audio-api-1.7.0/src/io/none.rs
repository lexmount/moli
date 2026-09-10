use std::thread;
use std::time::{Duration, Instant};

use super::{
    AudioBackendError, AudioBackendErrorKind, AudioBackendManager, BackendResult, RenderThreadInit,
};

use crate::buffer::AudioBuffer;
use crate::context::AudioContextOptions;
use crate::media_devices::MediaDeviceInfo;
use crate::render::RenderThread;
use crate::{MAX_CHANNELS, RENDER_QUANTUM_SIZE};

use crossbeam_channel::{Receiver, Sender};

enum NoneBackendMessage {
    Resume,
    Suspend,
    Close,
}

#[derive(Clone)]
pub(crate) struct NoneBackend {
    sender: Sender<NoneBackendMessage>,
    sample_rate: f32,
}

impl NoneBackend {
    /// Creates a mock backend to be used as tombstones
    pub(crate) fn void() -> Self {
        Self {
            sample_rate: 0.,
            sender: crossbeam_channel::bounded(0).0,
        }
    }
}

struct Callback {
    receiver: Receiver<NoneBackendMessage>,
    render_thread: RenderThread,
    sample_rate: f32,
    running: bool,
}

impl Callback {
    fn run(mut self) {
        let buffer_size = RENDER_QUANTUM_SIZE; // TODO Latency Category
        let mut buffer = vec![0.; buffer_size * MAX_CHANNELS];
        let interval = Duration::from_secs_f32(buffer_size as f32 / self.sample_rate);

        // For an isochronous callback we must calculate the deadline every render quantum
        let mut deadline = Instant::now().checked_add(interval).unwrap();

        loop {
            // poll the receiver as long as the deadline is in the future
            while let Ok(msg) = self.receiver.recv_deadline(deadline) {
                match msg {
                    NoneBackendMessage::Close => return,
                    NoneBackendMessage::Resume => {
                        self.running = true;
                        deadline = Instant::now().checked_add(interval).unwrap();
                        break; // start processing right away
                    }
                    NoneBackendMessage::Suspend => self.running = false,
                }
            }

            if self.running {
                self.render_thread.render(&mut buffer[..]);
            }

            deadline = deadline.checked_add(interval).unwrap();
        }
    }
}

impl AudioBackendManager for NoneBackend {
    /// Setup a new output stream (speakers)
    fn build_output(
        options: AudioContextOptions,
        render_thread_init: RenderThreadInit,
    ) -> BackendResult<Self>
    where
        Self: Sized,
    {
        let sample_rate = options.sample_rate.unwrap_or(48000.);

        let RenderThreadInit {
            state,
            startup_pending,
            frames_played,
            stats,
            ctrl_msg_recv,
            event_send,
        } = render_thread_init;

        let mut render_thread = RenderThread::new(
            sample_rate,
            MAX_CHANNELS,
            ctrl_msg_recv,
            state,
            frames_played,
            stats,
            event_send,
        );
        render_thread.set_startup_pending(startup_pending);
        render_thread.spawn_garbage_collector_thread();

        // Use a bounded channel for real-time safety. A maximum of 32 control messages (resume,
        // suspend, ..) will be handled per render quantum. The control thread will block when the
        // capacity is reached.
        let (sender, receiver) = crossbeam_channel::bounded(32);

        // todo: pass buffer size and sample rate
        let callback = Callback {
            render_thread,
            receiver,
            sample_rate,
            running: true,
        };

        thread::spawn(move || callback.run());

        Ok(Self {
            sender,
            sample_rate,
        })
    }

    /// Setup a new input stream (microphone capture)
    fn build_input(
        _options: AudioContextOptions,
        _number_of_channels: Option<u32>,
    ) -> BackendResult<(Self, Receiver<AudioBuffer>)>
    where
        Self: Sized,
    {
        Err(AudioBackendError::new(
            AudioBackendErrorKind::NotSupported,
            "none",
            "build_input",
            "The none backend does not support audio input",
        ))
    }

    /// Resume or start the stream
    fn resume(&self) -> BackendResult<bool> {
        self.sender
            .send(NoneBackendMessage::Resume)
            .map(|_| true)
            .map_err(|_| {
                AudioBackendError::new(
                    AudioBackendErrorKind::BackendSpecific,
                    "none",
                    "resume",
                    "The render thread is no longer available",
                )
            })
    }

    /// Suspend the stream
    fn suspend(&self) -> BackendResult<bool> {
        self.sender
            .send(NoneBackendMessage::Suspend)
            .map(|_| true)
            .map_err(|_| {
                AudioBackendError::new(
                    AudioBackendErrorKind::BackendSpecific,
                    "none",
                    "suspend",
                    "The render thread is no longer available",
                )
            })
    }

    /// Close the stream, freeing all resources. It cannot be started again after closing.
    fn close(&self) -> BackendResult<()> {
        self.sender.send(NoneBackendMessage::Close).map_err(|_| {
            AudioBackendError::new(
                AudioBackendErrorKind::BackendSpecific,
                "none",
                "close",
                "The render thread is no longer available",
            )
        })
    }

    /// Sample rate of the stream
    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Number of channels of the stream
    fn number_of_channels(&self) -> usize {
        MAX_CHANNELS
    }

    /// Output latency of the stream in seconds
    ///
    /// This is the difference between the time the backend acquires the data in the callback and
    /// the listener can hear the sound.
    fn output_latency(&self) -> BackendResult<f64> {
        Ok(0.)
    }

    /// The audio output device
    fn sink_id(&self) -> &str {
        "none"
    }

    fn enumerate_devices_sync() -> BackendResult<Vec<MediaDeviceInfo>>
    where
        Self: Sized,
    {
        Ok(vec![])
    }
}
