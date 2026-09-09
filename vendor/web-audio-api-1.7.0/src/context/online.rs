//! The `AudioContext` type and constructor options
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(feature = "diagnostics")]
use crate::context::{AudioBackendDiagnostics, AudioContextDiagnostics};
use crate::context::{AudioContextState, BaseAudioContext, ConcreteBaseAudioContext};
#[cfg(feature = "diagnostics")]
use crate::events::EventPayload;
use crate::events::{EventDispatch, EventHandler, EventLoop, EventType};
use crate::io::{self, AudioBackendManager, ControlThreadInit, NoneBackend, RenderThreadInit};
use crate::media_devices::{enumerate_devices_sync, MediaDeviceInfoKind};
use crate::media_streams::{MediaStream, MediaStreamTrack};
use crate::message::{ControlMessage, OneshotNotify};
use crate::node::{self, AudioNodeOptions};
use crate::render::graph::Graph;
use crate::MediaElement;
use crate::{is_valid_sample_rate, AudioPlaybackStats, AudioRenderCapacity, Event};

use futures_channel::oneshot;

/// Check if the provided sink_id is available for playback
///
/// It should be "", "none" or a valid output `sinkId` returned from [`enumerate_devices_sync`]
fn is_valid_sink_id(sink_id: &str) -> bool {
    if sink_id.is_empty() || sink_id == "none" {
        true
    } else {
        enumerate_devices_sync()
            .into_iter()
            .filter(|d| d.kind() == MediaDeviceInfoKind::AudioOutput)
            .any(|d| d.device_id() == sink_id)
    }
}

#[derive(Debug)]
enum AudioContextError {
    SinkNotFound { sink_id: String },
    InvalidSampleRate { sample_rate: f32 },
    Backend { error: io::AudioBackendError },
}

impl std::fmt::Display for AudioContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SinkNotFound { sink_id } => {
                write!(f, "NotFoundError - Invalid sinkId: {sink_id:?}")
            }
            Self::InvalidSampleRate { sample_rate } => {
                write!(
                    f,
                    "NotSupportedError - Invalid sample rate: {sample_rate}, should be in the range [3000.0, 768000.0]"
                )
            }
            Self::Backend { error } => write!(f, "InvalidStateError - {error}"),
        }
    }
}

impl Error for AudioContextError {}

impl From<io::AudioBackendError> for AudioContextError {
    fn from(error: io::AudioBackendError) -> Self {
        Self::Backend { error }
    }
}

/// Identify the type of playback, which affects tradeoffs
/// between audio output latency and power consumption
#[derive(Copy, Clone, Debug, Default)]
pub enum AudioContextLatencyCategory {
    /// Balance audio output latency and power consumption.
    Balanced,
    /// Provide the lowest audio output latency possible without glitching. This is the default.
    #[default]
    Interactive,
    /// Prioritize sustained playback without interruption over audio output latency.
    ///
    /// Lowest power consumption.
    Playback,
    /// Specify the number of seconds of latency
    ///
    /// This latency is not guaranteed to be applied, it depends on the audio hardware capabilities
    Custom(f64),
}

#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
/// This allows users to ask for a particular render quantum size.
///
/// Currently, only the default value is available
#[derive(Default)]
pub enum AudioContextRenderSizeCategory {
    /// The default value of 128 frames
    #[default]
    Default,
}

/// Specify the playback configuration for the [`AudioContext`] constructor.
///
/// All fields are optional and will default to the value best suited for interactive playback on
/// your hardware configuration.
///
/// For future compatibility, it is best to construct a default implementation of this struct and
/// set the fields you would like to override:
/// ```
/// use web_audio_api::context::AudioContextOptions;
///
/// // Request a sample rate of 44.1 kHz, leave other fields to their default values
/// let opts = AudioContextOptions {
///     sample_rate: Some(44100.),
///     ..AudioContextOptions::default()
/// };
#[derive(Clone, Debug, Default)]
pub struct AudioContextOptions {
    /// Identify the type of playback, which affects tradeoffs between audio output latency and
    /// power consumption.
    pub latency_hint: AudioContextLatencyCategory,

    /// Sample rate of the audio context and audio output hardware. Use `None` for a default value.
    pub sample_rate: Option<f32>,

    /// The audio output device
    /// - use `""` for the default audio output device
    /// - use `"none"` to process the audio graph without playing through an audio output device.
    /// - use `"sinkId"` to use the specified audio sink id, obtained with [`enumerate_devices_sync`]
    pub sink_id: String,

    /// Option to request a default, optimized or specific render quantum size. It is a hint that might not be honored.
    pub render_size_hint: AudioContextRenderSizeCategory,
}

/// This interface represents an audio graph whose `AudioDestinationNode` is routed to a real-time
/// output device that produces a signal directed at the user.
// the naming comes from the web audio specification
#[allow(clippy::module_name_repetitions)]
pub struct AudioContext {
    /// represents the underlying `BaseAudioContext`
    base: ConcreteBaseAudioContext,
    /// audio backend (play/pause functionality)
    backend_manager: Mutex<Box<dyn AudioBackendManager>>,
    /// Provider for rendering performance metrics
    render_capacity: AudioRenderCapacity,
    /// Provider for playback statistics
    playback_stats: AudioPlaybackStats,
    /// true while the render thread has not yet processed its initial Startup message
    startup_pending: std::sync::Arc<AtomicBool>,
    /// Initializer for the render thread (when restart is required)
    render_thread_init: RenderThreadInit,
}

impl std::fmt::Debug for AudioContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioContext")
            .field("sink_id", &self.sink_id())
            .field("base_latency", &self.base_latency())
            .field("output_latency", &self.output_latency())
            .field("base", &self.base())
            .finish_non_exhaustive()
    }
}

impl Drop for AudioContext {
    fn drop(&mut self) {
        // Continue playing the stream if the AudioContext goes out of scope
        if self.state() == AudioContextState::Running {
            let tombstone = Box::new(NoneBackend::void());
            let original = std::mem::replace(self.backend_manager.get_mut().unwrap(), tombstone);
            Box::leak(original);
        }
    }
}

impl BaseAudioContext for AudioContext {
    fn base(&self) -> &ConcreteBaseAudioContext {
        &self.base
    }
}

impl Default for AudioContext {
    fn default() -> Self {
        Self::new(AudioContextOptions::default())
    }
}

impl AudioContext {
    /// Creates and returns a new `AudioContext` object.
    ///
    /// This will play live audio on the default output device.
    ///
    /// ```no_run
    /// use web_audio_api::context::{AudioContext, AudioContextOptions};
    ///
    /// // Request a sample rate of 44.1 kHz and default latency (buffer size 128, if available)
    /// let opts = AudioContextOptions {
    ///     sample_rate: Some(44100.),
    ///     ..AudioContextOptions::default()
    /// };
    ///
    /// // Setup the audio context that will emit to your speakers
    /// let context = AudioContext::new(opts);
    ///
    /// // Alternatively, use the default constructor to get the best settings for your hardware
    /// // let context = AudioContext::default();
    /// ```
    ///
    /// # Panics
    ///
    /// The `AudioContext` constructor will panic when an invalid `sinkId` is provided in the
    /// `AudioContextOptions`, when the sample rate is outside the valid range [3000.0, 768000.0],
    /// or when the selected audio backend cannot create or start the output stream. Use
    /// [`Self::try_new`] to handle these errors without panicking.
    #[must_use]
    pub fn new(options: AudioContextOptions) -> Self {
        Self::try_new_inner(options).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Creates and returns a new `AudioContext` object.
    ///
    /// This will play live audio on the requested output device and returns backend errors instead
    /// of panicking when the stream cannot be created.
    ///
    /// # Errors
    ///
    /// Returns an error when the sink id is invalid, the sample rate is outside the valid range
    /// [3000.0, 768000.0], or when the selected audio backend cannot create or start the output
    /// stream.
    pub fn try_new(options: AudioContextOptions) -> Result<Self, Box<dyn Error>> {
        Self::try_new_inner(options).map_err(Into::into)
    }

    fn try_new_inner(options: AudioContextOptions) -> Result<Self, AudioContextError> {
        // https://webaudio.github.io/web-audio-api/#validating-sink-identifier
        if !is_valid_sink_id(&options.sink_id) {
            return Err(AudioContextError::SinkNotFound {
                sink_id: options.sink_id,
            });
        }

        // Validate sample_rate if provided
        // https://webaudio.github.io/web-audio-api/#sample-rates
        if let Some(sample_rate) = options.sample_rate {
            if !is_valid_sample_rate(sample_rate) {
                return Err(AudioContextError::InvalidSampleRate { sample_rate });
            }
        }

        // Set up the audio output thread
        let (control_thread_init, render_thread_init) = io::thread_init();
        let startup_pending = Arc::clone(&render_thread_init.startup_pending);
        let backend = io::build_output(options, render_thread_init.clone())?;

        let ControlThreadInit {
            state,
            frames_played,
            stats,
            ctrl_msg_send,
            event_send,
            event_recv,
        } = control_thread_init;

        // Construct the audio Graph and hand it to the render thread
        let (node_id_producer, node_id_consumer) = llq::Queue::new().split();
        let graph = Graph::new(node_id_producer);
        let message = ControlMessage::Startup { graph };
        ctrl_msg_send.send(message).unwrap();

        // Set up the event loop thread that handles the events spawned by the render thread
        let event_loop = EventLoop::new(event_recv);

        // Put everything together in the BaseAudioContext (shared with offline context)
        let base = ConcreteBaseAudioContext::new(
            backend.sample_rate(),
            backend.number_of_channels(),
            state,
            frames_played,
            ctrl_msg_send,
            event_send,
            event_loop.clone(),
            false,
            node_id_consumer,
        );

        // Setup AudioRenderCapacity for this context
        let render_capacity = AudioRenderCapacity::new(base.clone(), stats.clone());
        let playback_stats = AudioPlaybackStats::new(base.clone(), stats);

        // As the final step, spawn a thread for the event loop. If we do this earlier we may miss
        // event handling of the initial events that are emitted right after render thread
        // construction.
        event_loop.run_in_thread();

        Ok(Self {
            base,
            backend_manager: Mutex::new(backend),
            render_capacity,
            playback_stats,
            startup_pending,
            render_thread_init,
        })
    }

    /// This represents the number of seconds of processing latency incurred by
    /// the `AudioContext` passing the audio from the `AudioDestinationNode`
    /// to the audio subsystem.
    // We don't do any buffering between rendering the audio and sending
    // it to the audio subsystem, so this value is zero. (see Gecko)
    #[allow(clippy::unused_self)]
    #[must_use]
    pub fn base_latency(&self) -> f64 {
        0.
    }

    /// The estimation in seconds of audio output latency, i.e., the interval
    /// between the time the UA requests the host system to play a buffer and
    /// the time at which the first sample in the buffer is actually processed
    /// by the audio output device.
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn output_latency(&self) -> f64 {
        self.try_output_latency()
            .unwrap_or_else(|e| panic!("InvalidStateError - {e}"))
    }

    /// The estimation in seconds of audio output latency.
    ///
    /// # Errors
    ///
    /// Returns an error when the selected audio backend cannot query the output latency.
    fn try_output_latency(&self) -> Result<f64, Box<dyn Error>> {
        Ok(self.backend_manager.lock().unwrap().output_latency()?)
    }

    /// Identifier or the information of the current audio output device.
    ///
    /// The initial value is `""`, which means the default audio output device.
    #[allow(clippy::missing_panics_doc)]
    pub fn sink_id(&self) -> String {
        self.backend_manager.lock().unwrap().sink_id().to_owned()
    }

    /// Returns an [`AudioRenderCapacity`] instance associated with an AudioContext.
    #[must_use]
    pub fn render_capacity(&self) -> AudioRenderCapacity {
        self.render_capacity.clone()
    }

    /// Returns an [`AudioPlaybackStats`] instance associated with this `AudioContext`.
    #[must_use]
    pub fn playback_stats(&self) -> AudioPlaybackStats {
        self.playback_stats.clone()
    }

    /// Update the current audio output device.
    ///
    /// The provided `sink_id` string must match a device name `enumerate_devices_sync`.
    ///
    /// Supplying `"none"` for the `sink_id` will process the audio graph without playing through an
    /// audio output device.
    ///
    /// This function operates synchronously and might block the current thread. An async version
    /// is currently not implemented.
    #[allow(clippy::needless_collect, clippy::missing_panics_doc)]
    pub fn set_sink_id_sync(&self, sink_id: String) -> Result<(), Box<dyn Error>> {
        log::debug!("SinkChange requested");
        if self.sink_id() == sink_id {
            log::debug!("SinkChange: no-op");
            return Ok(()); // sink is already active
        }

        if !is_valid_sink_id(&sink_id) {
            Err(format!("NotFoundError: invalid sinkId {sink_id}"))?;
        };

        log::debug!("SinkChange: locking backend manager");
        let mut backend_manager_guard = self.backend_manager.lock().unwrap();
        let original_state = self.state();
        if original_state == AudioContextState::Closed {
            log::debug!("SinkChange: context is closed");
            return Ok(());
        }

        // Acquire exclusive lock on ctrl msg sender
        log::debug!("SinkChange: locking message channel");
        let ctrl_msg_send = self.base.lock_control_msg_sender();

        // Flush out the ctrl msg receiver, cache
        let mut pending_msgs: Vec<_> = self.render_thread_init.ctrl_msg_recv.try_iter().collect();

        // Acquire the active audio graph from the current render thread, shutting it down
        let graph = if matches!(pending_msgs.first(), Some(ControlMessage::Startup { .. })) {
            // Handle the edge case where the previous backend was suspended for its entire lifetime.
            // In this case, the `Startup` control message was never processed.
            log::debug!("SinkChange: recover unstarted graph");

            let msg = pending_msgs.remove(0);
            match msg {
                ControlMessage::Startup { graph } => graph,
                _ => unreachable!(),
            }
        } else {
            // Acquire the audio graph from the current render thread, shutting it down
            log::debug!("SinkChange: recover graph from render thread");

            let (graph_send, graph_recv) = crossbeam_channel::bounded(1);
            let message = ControlMessage::CloseAndRecycle { sender: graph_send };
            ctrl_msg_send.send(message).unwrap();
            if original_state == AudioContextState::Suspended {
                // We must wake up the render thread to be able to handle the shutdown.
                // No new audio will be produced because it will receive the shutdown command first.
                backend_manager_guard.resume()?;
            }
            graph_recv.recv().unwrap()
        };

        log::debug!("SinkChange: closing audio stream");
        backend_manager_guard.close()?;

        // hotswap the backend
        let options = AudioContextOptions {
            sample_rate: Some(self.sample_rate()),
            latency_hint: AudioContextLatencyCategory::default(), // todo reuse existing setting
            sink_id,
            render_size_hint: AudioContextRenderSizeCategory::default(), // todo reuse existing setting
        };
        log::debug!("SinkChange: starting audio stream");
        *backend_manager_guard = io::build_output(options, self.render_thread_init.clone())?;

        // if the previous backend state was suspend, suspend the new one before shipping the graph
        if original_state == AudioContextState::Suspended {
            log::debug!("SinkChange: suspending audio stream");
            backend_manager_guard.suspend()?;
        }

        // send the audio graph to the new render thread
        let message = ControlMessage::Startup { graph };
        ctrl_msg_send.send(message).unwrap();

        // flush the cached msgs
        pending_msgs
            .into_iter()
            .for_each(|m| self.base().send_control_msg(m));

        // explicitly release the lock to prevent concurrent render threads
        drop(backend_manager_guard);

        // trigger event when all the work is done
        let _ = self.base.send_event(EventDispatch::sink_change());

        log::debug!("SinkChange: done");
        Ok(())
    }

    /// Register callback to run when the audio sink has changed
    ///
    /// Only a single event handler is active at any time. Calling this method multiple times will
    /// override the previous event handler.
    pub fn set_onsinkchange<F: FnMut(Event) + Send + 'static>(&self, mut callback: F) {
        let callback = move |_| {
            callback(Event {
                type_: "sinkchange",
            })
        };

        self.base().set_event_handler(
            EventType::SinkChange,
            EventHandler::Multiple(Box::new(callback)),
        );
    }

    /// Unset the callback to run when the audio sink has changed
    pub fn clear_onsinkchange(&self) {
        self.base().clear_event_handler(EventType::SinkChange);
    }

    /// Request a structured diagnostic report of the audio context.
    ///
    /// The report is collected asynchronously: backend details are captured on the control thread,
    /// while render thread and graph details are captured in the realtime render thread. The
    /// callback is invoked once on the event loop thread.
    ///
    /// This API is available with the `diagnostics` crate feature.
    #[cfg(feature = "diagnostics")]
    #[allow(clippy::missing_panics_doc)]
    pub fn run_diagnostics<F: Fn(AudioContextDiagnostics) + Send + 'static>(&self, callback: F) {
        let backend = {
            let backend = self.backend_manager.lock().unwrap();
            AudioBackendDiagnostics {
                name: backend.name().to_string(),
                sink_id: backend.sink_id().to_string(),
                output_latency: backend.output_latency().ok(),
            }
        };

        let callback = move |v| match v {
            EventPayload::Diagnostics(v) => {
                callback(v);
            }
            _ => unreachable!(),
        };

        self.base().set_event_handler(
            EventType::Diagnostics,
            EventHandler::Once(Box::new(callback)),
        );

        self.base()
            .send_control_msg(ControlMessage::RunDiagnostics { backend });
    }

    /// Suspends the progression of time in the audio context.
    ///
    /// This will temporarily halt audio hardware access and reducing CPU/battery usage in the
    /// process.
    ///
    /// # Panics
    ///
    /// Will panic if:
    ///
    /// * The audio device is not available
    /// * For a `BackendSpecificError`
    pub async fn suspend(&self) {
        // Don't lock the backend manager because we can't hold is across the await point
        log::debug!("Suspend called");

        let state = self.state();
        if state == AudioContextState::Closed {
            log::debug!("Suspend no-op - context is closed");
            return;
        }

        if state != AudioContextState::Running && !self.startup_pending.load(Ordering::Acquire) {
            log::debug!("Suspend no-op - context is not running");
            return;
        }

        // Pause rendering via a control message
        let (sender, receiver) = oneshot::channel();
        let notify = OneshotNotify::Async(sender);
        self.base
            .suspend_control_msgs(ControlMessage::Suspend { notify });

        // Wait for the render thread to have processed the suspend message.
        // The AudioContextState will be updated by the render thread.
        log::debug!("Suspending audio graph, waiting for signal..");
        receiver.await.unwrap();

        // Then ask the audio host to suspend the stream
        log::debug!("Suspended audio graph. Suspending audio stream..");
        self.backend_manager
            .lock()
            .unwrap()
            .suspend()
            .unwrap_or_else(|e| panic!("InvalidStateError - {e}"));

        log::debug!("Suspended audio stream");
    }

    /// Resumes the progression of time in an audio context that has previously been
    /// suspended/paused.
    ///
    /// # Panics
    ///
    /// Will panic if:
    ///
    /// * The audio device is not available
    /// * For a `BackendSpecificError`
    pub async fn resume(&self) {
        let (sender, receiver) = oneshot::channel();

        {
            // Lock the backend manager mutex to avoid concurrent calls
            log::debug!("Resume called, locking backend manager");
            let backend_manager_guard = self.backend_manager.lock().unwrap();

            if self.state() != AudioContextState::Suspended {
                log::debug!("Resume no-op - context is not suspended");
                return;
            }

            // Ask the audio host to resume the stream
            backend_manager_guard
                .resume()
                .unwrap_or_else(|e| panic!("InvalidStateError - {e}"));

            // Then, ask to resume rendering via a control message
            log::debug!("Resumed audio stream, waking audio graph");
            let notify = OneshotNotify::Async(sender);
            self.base
                .resume_control_msgs(ControlMessage::Resume { notify });

            // Drop the Mutex guard so we won't hold it across an await point
        }

        // Wait for the render thread to have processed the resume message
        // The AudioContextState will be updated by the render thread.
        receiver.await.unwrap();
        log::debug!("Resumed audio graph");
    }

    /// Closes the `AudioContext`, releasing the system resources being used.
    ///
    /// This will not automatically release all `AudioContext`-created objects, but will suspend
    /// the progression of the currentTime, and stop processing audio data.
    ///
    /// # Panics
    ///
    /// Will panic when this function is called multiple times
    pub async fn close(&self) {
        // Don't lock the backend manager because we can't hold is across the await point
        log::debug!("Close called");

        if self.state() == AudioContextState::Closed {
            log::debug!("Close no-op - context is already closed");
            return;
        }

        // Stop AudioRenderCapacity before closing so no capacity events are queued during shutdown.
        self.render_capacity.stop();

        if self.state() == AudioContextState::Running {
            // First, stop rendering via a control message
            let (sender, receiver) = oneshot::channel();
            let notify = OneshotNotify::Async(sender);
            self.base.send_control_msg(ControlMessage::Close { notify });

            // Wait for the render thread to have processed the suspend message.
            // The AudioContextState will be updated by the render thread.
            log::debug!("Suspending audio graph, waiting for signal..");
            receiver.await.unwrap();
        } else {
            // if the context is not running, change the state manually
            self.base.set_state(AudioContextState::Closed);
        }

        // Then ask the audio host to close the stream
        log::debug!("Suspended audio graph. Closing audio stream..");
        self.backend_manager
            .lock()
            .unwrap()
            .close()
            .unwrap_or_else(|e| panic!("InvalidStateError - {e}"));

        log::debug!("Closed audio stream");
    }

    /// Suspends the progression of time in the audio context.
    ///
    /// This will temporarily halt audio hardware access and reducing CPU/battery usage in the
    /// process.
    ///
    /// This function operates synchronously and blocks the current thread until the audio thread
    /// has stopped processing.
    ///
    /// # Panics
    ///
    /// Will panic if:
    ///
    /// * The audio device is not available
    /// * For a `BackendSpecificError`
    pub fn suspend_sync(&self) {
        // Lock the backend manager mutex to avoid concurrent calls
        log::debug!("Suspend_sync called, locking backend manager");
        let backend_manager_guard = self.backend_manager.lock().unwrap();

        let state = self.state();
        if state == AudioContextState::Closed {
            log::debug!("Suspend_sync no-op - context is closed");
            return;
        }

        if state != AudioContextState::Running && !self.startup_pending.load(Ordering::Acquire) {
            log::debug!("Suspend_sync no-op - context is not running");
            return;
        }

        // Pause rendering via a control message
        let (sender, receiver) = crossbeam_channel::bounded(0);
        let notify = OneshotNotify::Sync(sender);
        self.base
            .suspend_control_msgs(ControlMessage::Suspend { notify });

        // Wait for the render thread to have processed the suspend message.
        // The AudioContextState will be updated by the render thread.
        log::debug!("Suspending audio graph, waiting for signal..");
        receiver.recv().ok();

        // Then ask the audio host to suspend the stream
        log::debug!("Suspended audio graph. Suspending audio stream..");
        backend_manager_guard
            .suspend()
            .unwrap_or_else(|e| panic!("InvalidStateError - {e}"));

        log::debug!("Suspended audio stream");
    }

    /// Resumes the progression of time in an audio context that has previously been
    /// suspended/paused.
    ///
    /// This function operates synchronously and blocks the current thread until the audio thread
    /// has started processing again.
    ///
    /// # Panics
    ///
    /// Will panic if:
    ///
    /// * The audio device is not available
    /// * For a `BackendSpecificError`
    pub fn resume_sync(&self) {
        // Lock the backend manager mutex to avoid concurrent calls
        log::debug!("Resume_sync called, locking backend manager");
        let backend_manager_guard = self.backend_manager.lock().unwrap();

        if self.state() != AudioContextState::Suspended {
            log::debug!("Resume no-op - context is not suspended");
            return;
        }

        // Ask the audio host to resume the stream
        backend_manager_guard
            .resume()
            .unwrap_or_else(|e| panic!("InvalidStateError - {e}"));

        // Then, ask to resume rendering via a control message
        log::debug!("Resumed audio stream, waking audio graph");
        let (sender, receiver) = crossbeam_channel::bounded(0);
        let notify = OneshotNotify::Sync(sender);
        self.base
            .resume_control_msgs(ControlMessage::Resume { notify });

        // Wait for the render thread to have processed the resume message
        // The AudioContextState will be updated by the render thread.
        receiver.recv().ok();
        log::debug!("Resumed audio graph");
    }

    /// Closes the `AudioContext`, releasing the system resources being used.
    ///
    /// This will not automatically release all `AudioContext`-created objects, but will suspend
    /// the progression of the currentTime, and stop processing audio data.
    ///
    /// This function operates synchronously and blocks the current thread until the audio thread
    /// has stopped processing.
    ///
    /// # Panics
    ///
    /// Will panic when this function is called multiple times
    pub fn close_sync(&self) {
        // Lock the backend manager mutex to avoid concurrent calls
        log::debug!("Close_sync called, locking backend manager");
        let backend_manager_guard = self.backend_manager.lock().unwrap();

        if self.state() == AudioContextState::Closed {
            log::debug!("Close no-op - context is already closed");
            return;
        }

        // Stop AudioRenderCapacity before closing so no capacity events are queued during shutdown.
        self.render_capacity.stop();

        // First, stop rendering via a control message
        if self.state() == AudioContextState::Running {
            let (sender, receiver) = crossbeam_channel::bounded(0);
            let notify = OneshotNotify::Sync(sender);
            self.base.send_control_msg(ControlMessage::Close { notify });

            // Wait for the render thread to have processed the suspend message.
            // The AudioContextState will be updated by the render thread.
            log::debug!("Suspending audio graph, waiting for signal..");
            receiver.recv().ok();
        } else {
            // if the context is not running, change the state manually
            self.base.set_state(AudioContextState::Closed);
        }

        // Then ask the audio host to close the stream
        log::debug!("Suspended audio graph. Closing audio stream..");
        backend_manager_guard
            .close()
            .unwrap_or_else(|e| panic!("InvalidStateError - {e}"));

        log::debug!("Closed audio stream");
    }

    /// Creates a [`MediaStreamAudioSourceNode`](node::MediaStreamAudioSourceNode) from a
    /// [`MediaStream`]
    #[must_use]
    pub fn create_media_stream_source(
        &self,
        media: &MediaStream,
    ) -> node::MediaStreamAudioSourceNode {
        let opts = node::MediaStreamAudioSourceOptions {
            media_stream: media,
        };
        node::MediaStreamAudioSourceNode::new(self, opts)
    }

    /// Creates a [`MediaStreamAudioDestinationNode`](node::MediaStreamAudioDestinationNode)
    #[must_use]
    pub fn create_media_stream_destination(&self) -> node::MediaStreamAudioDestinationNode {
        let opts = AudioNodeOptions::default();
        node::MediaStreamAudioDestinationNode::new(self, opts)
    }

    /// Creates a [`MediaStreamTrackAudioSourceNode`](node::MediaStreamTrackAudioSourceNode) from a
    /// [`MediaStreamTrack`]
    #[must_use]
    pub fn create_media_stream_track_source(
        &self,
        media: &MediaStreamTrack,
    ) -> node::MediaStreamTrackAudioSourceNode {
        let opts = node::MediaStreamTrackAudioSourceOptions {
            media_stream_track: media,
        };
        node::MediaStreamTrackAudioSourceNode::new(self, opts)
    }

    /// Creates a [`MediaElementAudioSourceNode`](node::MediaElementAudioSourceNode) from a
    /// [`MediaElement`]
    #[must_use]
    pub fn create_media_element_source(
        &self,
        media_element: &mut MediaElement,
    ) -> node::MediaElementAudioSourceNode {
        let opts = node::MediaElementAudioSourceOptions { media_element };
        node::MediaElementAudioSourceNode::new(self, opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "diagnostics")]
    use crate::context::DESTINATION_NODE_ID;
    use futures::executor;

    #[test]
    fn test_suspend_resume_close() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };

        // construct with 'none' sink_id
        let context = AudioContext::new(options);

        // Ensure startup has been processed before testing suspend/resume transitions.
        executor::block_on(context.resume());
        assert_eq!(context.state(), AudioContextState::Running);

        executor::block_on(context.suspend());
        assert_eq!(context.state(), AudioContextState::Suspended);
        let time1 = context.current_time();
        assert!(time1 >= 0.);

        // allow some time to progress
        std::thread::sleep(std::time::Duration::from_millis(1));
        let time2 = context.current_time();
        assert_eq!(time1, time2); // no progression of time

        executor::block_on(context.resume());
        assert_eq!(context.state(), AudioContextState::Running);

        // allow some time to progress
        std::thread::sleep(std::time::Duration::from_millis(1));

        let time3 = context.current_time();
        assert!(time3 > time2); // time is progressing

        executor::block_on(context.close());
        assert_eq!(context.state(), AudioContextState::Closed);

        let time4 = context.current_time();

        // allow some time to progress
        std::thread::sleep(std::time::Duration::from_millis(1));

        let time5 = context.current_time();
        assert_eq!(time5, time4); // no progression of time
    }

    #[test]
    fn test_suspend_during_startup() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };

        let context = AudioContext::new(options);

        executor::block_on(context.suspend());
        assert_eq!(context.state(), AudioContextState::Suspended);

        let time1 = context.current_time();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let time2 = context.current_time();
        assert_eq!(time1, time2);
    }

    #[test]
    fn test_suspend_sync_during_startup() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };

        let context = AudioContext::new(options);

        context.suspend_sync();
        assert_eq!(context.state(), AudioContextState::Suspended);

        let time1 = context.current_time();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let time2 = context.current_time();
        assert_eq!(time1, time2);
    }

    fn require_send_sync<T: Send + Sync>(_: T) {}

    #[test]
    fn test_all_futures_thread_safe() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);

        require_send_sync(context.suspend());
        require_send_sync(context.resume());
        require_send_sync(context.close());
    }

    #[test]
    fn test_try_new_invalid_sample_rate() {
        let options = AudioContextOptions {
            sample_rate: Some(0.),
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };

        let result = AudioContext::try_new(options);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Invalid sample rate"));
    }

    #[test]
    #[should_panic]
    fn test_invalid_sink_id() {
        let options = AudioContextOptions {
            sink_id: "invalid".into(),
            ..AudioContextOptions::default()
        };
        let _ = AudioContext::new(options);
    }

    #[test]
    fn test_try_new_invalid_sink_id() {
        let options = AudioContextOptions {
            sink_id: "invalid".into(),
            ..AudioContextOptions::default()
        };

        let error = AudioContext::try_new(options).unwrap_err();
        assert_eq!(
            error.to_string(),
            "NotFoundError - Invalid sinkId: \"invalid\""
        );
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn test_run_diagnostics_returns_structured_output() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);
        let (sender, receiver) = std::sync::mpsc::channel();

        context.run_diagnostics(move |diagnostics| {
            sender.send(diagnostics).unwrap();
        });

        let diagnostics = receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        assert!(diagnostics.backend.name.contains("NoneBackend"));
        assert_eq!(diagnostics.backend.sink_id, "none");
        assert_eq!(diagnostics.backend.output_latency, Some(0.));
        assert_eq!(diagnostics.render_thread.sample_rate, context.sample_rate());
        assert_eq!(
            diagnostics.render_thread.number_of_channels,
            crate::MAX_CHANNELS
        );
        assert!(diagnostics.graph.active);
        assert_eq!(diagnostics.graph.node_count, diagnostics.graph.nodes.len());
        assert_eq!(diagnostics.graph.edge_count, 0);
        assert!(diagnostics.graph.in_cycle.is_empty());
        assert!(diagnostics.graph.cycle_breakers.is_empty());
        assert!(diagnostics
            .graph
            .nodes
            .iter()
            .any(|node| node.id == DESTINATION_NODE_ID.0
                && node.inputs == node.input_channels.len()
                && node.outputs == node.output_channels.len()));
    }
}
