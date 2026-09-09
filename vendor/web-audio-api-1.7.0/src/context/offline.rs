//! The `OfflineAudioContext` type

use std::sync::atomic::{AtomicU64, AtomicU8};
use std::sync::{Arc, Mutex};

use crate::buffer::AudioBuffer;
use crate::context::{AudioContextState, BaseAudioContext, ConcreteBaseAudioContext};
use crate::events::{
    Event, EventDispatch, EventHandler, EventPayload, EventType, OfflineAudioCompletionEvent,
};
use crate::render::RenderThread;
use crate::stats::AudioStats;
use crate::{
    assert_valid_buffer_length, assert_valid_number_of_channels, assert_valid_sample_rate,
    RENDER_QUANTUM_SIZE,
};

use crate::events::EventLoop;
use futures_channel::{mpsc, oneshot};
use futures_util::SinkExt as _;

pub(crate) type OfflineAudioContextCallback =
    dyn FnOnce(&mut OfflineAudioContext) + Send + Sync + 'static;

/// The `OfflineAudioContext` doesn't render the audio to the device hardware; instead, it generates
/// it, as fast as it can, and outputs the result to an `AudioBuffer`.
// the naming comes from the web audio specification
#[allow(clippy::module_name_repetitions)]
pub struct OfflineAudioContext {
    /// represents the underlying `BaseAudioContext`
    base: ConcreteBaseAudioContext,
    /// the size of the buffer in sample-frames
    length: usize,
    /// actual renderer of the audio graph, can only be called once
    renderer: Mutex<Option<OfflineAudioContextRenderer>>,
    /// channel to notify resume actions on the rendering
    resume_sender: mpsc::Sender<()>,
}

impl std::fmt::Debug for OfflineAudioContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineAudioContext")
            .field("length", &self.length())
            .field("base", &self.base())
            .finish_non_exhaustive()
    }
}

struct OfflineAudioContextRenderer {
    /// the rendering 'thread', fully controlled by the offline context
    renderer: RenderThread,
    /// sorted list of promises to resolve at certain render quanta (via `suspend`)
    suspend_promises: Vec<(usize, oneshot::Sender<()>)>,
    /// sorted list of callbacks to run at certain render quanta (via `suspend_sync`)
    suspend_callbacks: Vec<(usize, Box<OfflineAudioContextCallback>)>,
    /// channel to listen for `resume` calls on a suspended context
    resume_receiver: mpsc::Receiver<()>,
    /// event loop to run after each render quantum
    event_loop: EventLoop,
}

impl BaseAudioContext for OfflineAudioContext {
    fn base(&self) -> &ConcreteBaseAudioContext {
        &self.base
    }
}

impl OfflineAudioContext {
    /// Creates an `OfflineAudioContext` instance
    ///
    /// # Arguments
    ///
    /// * `channels` - number of output channels to render
    /// * `length` - length of the rendering audio buffer
    /// * `sample_rate` - output sample rate
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn new(number_of_channels: usize, length: usize, sample_rate: f32) -> Self {
        assert_valid_number_of_channels(number_of_channels);
        assert_valid_buffer_length(length);
        assert_valid_sample_rate(sample_rate);

        // communication channel to the render thread,
        // unbounded is fine because it does not need to be realtime safe
        let (sender, receiver) = crossbeam_channel::unbounded();

        let (node_id_producer, node_id_consumer) = llq::Queue::new().split();
        let graph = crate::render::graph::Graph::new(node_id_producer);
        let message = crate::message::ControlMessage::Startup { graph };
        sender.send(message).unwrap();

        // track number of frames - synced from render thread to control thread
        let frames_played = Arc::new(AtomicU64::new(0));
        let frames_played_clone = Arc::clone(&frames_played);
        let state = Arc::new(AtomicU8::new(AudioContextState::Suspended as u8));
        let state_clone = Arc::clone(&state);

        // Communication channel for events from the render thread to the control thread.
        // Use an unbounded channel because we do not require real-time safety.
        let (event_send, event_recv) = crossbeam_channel::unbounded();
        let event_loop = EventLoop::new(event_recv);

        // setup the render 'thread', which will run inside the control thread
        let renderer = RenderThread::new(
            sample_rate,
            number_of_channels,
            receiver,
            state_clone,
            frames_played_clone,
            AudioStats::new(),
            event_send.clone(),
        );

        // first, setup the base audio context
        let base = ConcreteBaseAudioContext::new(
            sample_rate,
            number_of_channels,
            state,
            frames_played,
            sender,
            event_send,
            event_loop.clone(),
            true,
            node_id_consumer,
        );

        let (resume_sender, resume_receiver) = mpsc::channel(0);

        let renderer = OfflineAudioContextRenderer {
            renderer,
            suspend_promises: Vec::new(),
            suspend_callbacks: Vec::new(),
            resume_receiver,
            event_loop,
        };

        Self {
            base,
            length,
            renderer: Mutex::new(Some(renderer)),
            resume_sender,
        }
    }

    /// Given the current connections and scheduled changes, starts rendering audio.
    ///
    /// This function will block the current thread and returns the rendered `AudioBuffer`
    /// synchronously.
    ///
    /// This method will only adhere to scheduled suspensions via [`Self::suspend_sync`] and
    /// will ignore those provided via [`Self::suspend`].
    ///
    /// # Panics
    ///
    /// Panics if this method is called multiple times
    #[must_use]
    pub fn start_rendering_sync(&mut self) -> AudioBuffer {
        let renderer = self
            .renderer
            .lock()
            .unwrap()
            .take()
            .expect("InvalidStateError - Cannot call `startRendering` twice");

        let OfflineAudioContextRenderer {
            renderer,
            suspend_callbacks,
            event_loop,
            ..
        } = renderer;

        self.base.set_state(AudioContextState::Running);

        let result = renderer.render_audiobuffer_sync(self, suspend_callbacks, &event_loop);

        self.base.set_state(AudioContextState::Closed);
        let _ = self
            .base
            .send_event(EventDispatch::complete(result.clone()));

        // spin the event loop once more to handle the statechange/complete events
        event_loop.handle_pending_events();

        result
    }

    /// Given the current connections and scheduled changes, starts rendering audio.
    ///
    /// Rendering is purely CPU bound and contains no `await` points, so calling this method will
    /// block the executor until completion or until the context is suspended.
    ///
    /// This method will only adhere to scheduled suspensions via [`Self::suspend`] and will
    /// ignore those provided via [`Self::suspend_sync`].
    ///
    /// # Panics
    ///
    /// Panics if this method is called multiple times.
    pub async fn start_rendering(&self) -> AudioBuffer {
        // We are mixing async with a std Mutex, so be sure not to `await` while the lock is held
        let renderer = self
            .renderer
            .lock()
            .unwrap()
            .take()
            .expect("InvalidStateError - Cannot call `startRendering` twice");

        let OfflineAudioContextRenderer {
            renderer,
            suspend_promises,
            resume_receiver,
            event_loop,
            ..
        } = renderer;

        self.base.set_state(AudioContextState::Running);

        let result = renderer
            .render_audiobuffer(self.length, suspend_promises, resume_receiver, &event_loop)
            .await;

        self.base.set_state(AudioContextState::Closed);
        let _ = self
            .base
            .send_event(EventDispatch::complete(result.clone()));

        // spin the event loop once more to handle the statechange/complete events
        event_loop.handle_pending_events();

        result
    }

    /// get the length of rendering audio buffer
    // false positive: OfflineAudioContext is not const
    #[allow(clippy::missing_const_for_fn, clippy::unused_self)]
    #[must_use]
    pub fn length(&self) -> usize {
        self.length
    }

    #[track_caller]
    fn calculate_suspend_frame(&self, suspend_time: f64) -> usize {
        assert!(
            suspend_time >= 0.,
            "InvalidStateError: suspendTime cannot be negative"
        );
        assert!(
            suspend_time < self.length as f64 / self.sample_rate() as f64,
            "InvalidStateError: suspendTime cannot be greater than or equal to the total render duration"
        );
        (suspend_time * self.base.sample_rate() as f64 / RENDER_QUANTUM_SIZE as f64).ceil() as usize
    }

    /// Schedules a suspension of the time progression in the audio context at the specified time
    /// and returns a promise
    ///
    /// The specified time is quantized and rounded up to the render quantum size.
    ///
    /// # Panics
    ///
    /// Panics if the quantized frame number
    ///
    /// - is negative or
    /// - is less than or equal to the current time or
    /// - is greater than or equal to the total render duration or
    /// - is scheduled by another suspend for the same time
    ///
    /// # Example usage
    ///
    /// ```rust
    /// use futures::{executor, join};
    /// use futures::FutureExt as _;
    /// use std::sync::Arc;
    ///
    /// use web_audio_api::context::BaseAudioContext;
    /// use web_audio_api::context::OfflineAudioContext;
    /// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
    ///
    /// let context = Arc::new(OfflineAudioContext::new(1, 512, 44_100.));
    /// let context_clone = Arc::clone(&context);
    ///
    /// let suspend_promise = context.suspend(128. / 44_100.).then(|_| async move {
    ///     let mut src = context_clone.create_constant_source();
    ///     src.connect(&context_clone.destination());
    ///     src.start();
    ///     context_clone.resume().await;
    /// });
    ///
    /// let render_promise = context.start_rendering();
    ///
    /// let buffer = executor::block_on(async move { join!(suspend_promise, render_promise).1 });
    /// assert_eq!(buffer.number_of_channels(), 1);
    /// assert_eq!(buffer.length(), 512);
    /// ```
    pub async fn suspend(&self, suspend_time: f64) {
        let quantum = self.calculate_suspend_frame(suspend_time);

        let (sender, receiver) = oneshot::channel();

        // We are mixing async with a std Mutex, so be sure not to `await` while the lock is held
        {
            let mut lock = self.renderer.lock().unwrap();
            let renderer = lock
                .as_mut()
                .expect("InvalidStateError - cannot suspend when rendering has already started");

            let insert_pos = renderer
                .suspend_promises
                .binary_search_by_key(&quantum, |&(q, _)| q)
                .expect_err(
                    "InvalidStateError - cannot suspend multiple times at the same render quantum",
                );

            renderer
                .suspend_promises
                .insert(insert_pos, (quantum, sender));
        } // lock is dropped

        receiver.await.unwrap();
        self.base().set_state(AudioContextState::Suspended);
    }

    /// Schedules a suspension of the time progression in the audio context at the specified time
    /// and runs a callback.
    ///
    /// This is a synchronous version of [`Self::suspend`] that runs the provided callback at
    /// the `suspendTime`. The rendering resumes automatically after the callback has run, so there
    /// is no `resume_sync` method.
    ///
    /// The specified time is quantized and rounded up to the render quantum size.
    ///
    /// # Panics
    ///
    /// Panics if the quantized frame number
    ///
    /// - is negative or
    /// - is less than or equal to the current time or
    /// - is greater than or equal to the total render duration or
    /// - is scheduled by another suspend for the same time
    ///
    /// # Example usage
    ///
    /// ```rust
    /// use web_audio_api::context::BaseAudioContext;
    /// use web_audio_api::context::OfflineAudioContext;
    /// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
    ///
    /// let mut context = OfflineAudioContext::new(1, 512, 44_100.);
    ///
    /// context.suspend_sync(128. / 44_100., |context| {
    ///     let mut src = context.create_constant_source();
    ///     src.connect(&context.destination());
    ///     src.start();
    /// });
    ///
    /// let buffer = context.start_rendering_sync();
    /// assert_eq!(buffer.number_of_channels(), 1);
    /// assert_eq!(buffer.length(), 512);
    /// ```
    pub fn suspend_sync<F: FnOnce(&mut Self) + Send + Sync + 'static>(
        &mut self,
        suspend_time: f64,
        callback: F,
    ) {
        let quantum = self.calculate_suspend_frame(suspend_time);

        let mut lock = self.renderer.lock().unwrap();
        let renderer = lock
            .as_mut()
            .expect("InvalidStateError - cannot suspend when rendering has already started");

        let insert_pos = renderer
            .suspend_callbacks
            .binary_search_by_key(&quantum, |(q, _c)| *q)
            .expect_err(
                "InvalidStateError - cannot suspend multiple times at the same render quantum",
            );

        let boxed_callback = Box::new(|ctx: &mut OfflineAudioContext| {
            ctx.base().set_state(AudioContextState::Suspended);
            (callback)(ctx);
            ctx.base().set_state(AudioContextState::Running);
        });

        renderer
            .suspend_callbacks
            .insert(insert_pos, (quantum, boxed_callback));
    }

    /// Resumes the progression of the OfflineAudioContext's currentTime when it has been suspended
    ///
    /// # Panics
    ///
    /// Panics when the context is closed or rendering has not started
    pub async fn resume(&self) {
        self.base().set_state(AudioContextState::Running);
        self.resume_sender.clone().send(()).await.unwrap()
    }

    /// Register callback to run when the rendering has completed
    ///
    /// Only a single event handler is active at any time. Calling this method multiple times will
    /// override the previous event handler.
    #[allow(clippy::missing_panics_doc)]
    pub fn set_oncomplete<F: FnOnce(OfflineAudioCompletionEvent) + Send + 'static>(
        &self,
        callback: F,
    ) {
        let callback = move |v| match v {
            EventPayload::Complete(v) => {
                let event = OfflineAudioCompletionEvent {
                    rendered_buffer: v,
                    event: Event { type_: "complete" },
                };
                callback(event)
            }
            _ => unreachable!(),
        };

        self.base()
            .set_event_handler(EventType::Complete, EventHandler::Once(Box::new(callback)));
    }

    /// Unset the callback to run when the rendering has completed
    pub fn clear_oncomplete(&self) {
        self.base().clear_event_handler(EventType::Complete);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_eq::assert_float_eq;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::node::AudioNode;
    use crate::node::AudioScheduledSourceNode;

    #[test]
    fn test_sample_rate_length() {
        let context = OfflineAudioContext::new(1, 48000, 96000.);
        assert_float_eq!(context.sample_rate(), 96000., abs_all <= 0.);
        assert_eq!(context.length(), 48000);
    }

    #[test]
    fn render_empty_graph() {
        let mut context = OfflineAudioContext::new(2, 555, 44_100.);
        assert_eq!(context.state(), AudioContextState::Suspended);
        let buffer = context.start_rendering_sync();

        assert_eq!(context.length(), 555);

        assert_eq!(buffer.number_of_channels(), 2);
        assert_eq!(buffer.length(), 555);
        assert_float_eq!(buffer.get_channel_data(0), &[0.; 555][..], abs_all <= 0.);
        assert_float_eq!(buffer.get_channel_data(1), &[0.; 555][..], abs_all <= 0.);

        assert_eq!(context.state(), AudioContextState::Closed);
    }

    #[test]
    #[should_panic]
    fn render_twice_panics() {
        let mut context = OfflineAudioContext::new(2, 555, 44_100.);
        let _ = context.start_rendering_sync();
        let _ = context.start_rendering_sync();
    }

    #[test]
    fn test_suspend_sync() {
        use crate::node::ConstantSourceNode;
        use std::sync::OnceLock;

        let len = RENDER_QUANTUM_SIZE * 4;
        let sample_rate = 48000_f64;

        let mut context = OfflineAudioContext::new(1, len, sample_rate as f32);
        static SOURCE: OnceLock<ConstantSourceNode> = OnceLock::new();

        context.suspend_sync(RENDER_QUANTUM_SIZE as f64 / sample_rate, |context| {
            assert_eq!(context.state(), AudioContextState::Suspended);
            let mut src = context.create_constant_source();
            src.connect(&context.destination());
            src.start();
            SOURCE.set(src).unwrap();
        });

        context.suspend_sync((3 * RENDER_QUANTUM_SIZE) as f64 / sample_rate, |context| {
            assert_eq!(context.state(), AudioContextState::Suspended);
            SOURCE.get().unwrap().disconnect();
        });

        let output = context.start_rendering_sync();

        assert_float_eq!(
            output.get_channel_data(0)[..RENDER_QUANTUM_SIZE],
            &[0.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
        assert_float_eq!(
            output.get_channel_data(0)[RENDER_QUANTUM_SIZE..3 * RENDER_QUANTUM_SIZE],
            &[1.; 2 * RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
        assert_float_eq!(
            output.get_channel_data(0)[3 * RENDER_QUANTUM_SIZE..4 * RENDER_QUANTUM_SIZE],
            &[0.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn render_suspend_resume_async() {
        use futures::executor;
        use futures::join;
        use futures::FutureExt as _;

        let context = Arc::new(OfflineAudioContext::new(1, 512, 44_100.));
        let context_clone = Arc::clone(&context);

        let suspend_promise = context.suspend(128. / 44_100.).then(|_| async move {
            let mut src = context_clone.create_constant_source();
            src.connect(&context_clone.destination());
            src.start();
            context_clone.resume().await;
        });

        let render_promise = context.start_rendering();

        let buffer = executor::block_on(async move { join!(suspend_promise, render_promise).1 });

        assert_eq!(buffer.number_of_channels(), 1);
        assert_eq!(buffer.length(), 512);

        assert_float_eq!(
            buffer.get_channel_data(0)[..128],
            &[0.; 128][..],
            abs_all <= 0.
        );
        assert_float_eq!(
            buffer.get_channel_data(0)[128..],
            &[1.; 384][..],
            abs_all <= 0.
        );
    }

    #[test]
    #[should_panic]
    fn test_suspend_negative_panics() {
        let mut context = OfflineAudioContext::new(2, 128, 44_100.);
        context.suspend_sync(-1.0, |_| ());
    }

    #[test]
    #[should_panic]
    fn test_suspend_after_duration_panics() {
        let mut context = OfflineAudioContext::new(2, 128, 44_100.);
        context.suspend_sync(1.0, |_| ());
    }

    #[test]
    #[should_panic]
    fn test_suspend_after_render_panics() {
        let mut context = OfflineAudioContext::new(2, 128, 44_100.);
        let _ = context.start_rendering_sync();
        context.suspend_sync(0.0, |_| ());
    }

    #[test]
    #[should_panic]
    fn test_suspend_identical_frame_panics() {
        let mut context = OfflineAudioContext::new(2, 128, 44_100.);
        context.suspend_sync(0.0, |_| ());
        context.suspend_sync(0.0, |_| ());
    }

    #[test]
    fn test_onstatechange() {
        let mut context = OfflineAudioContext::new(2, 555, 44_100.);

        let changed = Arc::new(AtomicBool::new(false));
        let changed_clone = Arc::clone(&changed);
        context.set_onstatechange(move |_event| {
            changed_clone.store(true, Ordering::Relaxed);
        });

        let _ = context.start_rendering_sync();

        assert!(changed.load(Ordering::Relaxed));
    }

    #[test]
    fn test_onstatechange_async() {
        use futures::executor;

        let context = OfflineAudioContext::new(2, 555, 44_100.);

        let changed = Arc::new(AtomicBool::new(false));
        let changed_clone = Arc::clone(&changed);
        context.set_onstatechange(move |_event| {
            changed_clone.store(true, Ordering::Relaxed);
        });

        let _ = executor::block_on(context.start_rendering());

        assert!(changed.load(Ordering::Relaxed));
    }

    #[test]
    fn test_oncomplete() {
        let mut context = OfflineAudioContext::new(2, 555, 44_100.);

        let complete = Arc::new(AtomicBool::new(false));
        let complete_clone = Arc::clone(&complete);
        context.set_oncomplete(move |event| {
            assert_eq!(event.rendered_buffer.length(), 555);
            complete_clone.store(true, Ordering::Relaxed);
        });

        let _ = context.start_rendering_sync();

        assert!(complete.load(Ordering::Relaxed));
    }

    #[test]
    fn test_oncomplete_async() {
        use futures::executor;

        let context = OfflineAudioContext::new(2, 555, 44_100.);

        let complete = Arc::new(AtomicBool::new(false));
        let complete_clone = Arc::clone(&complete);
        context.set_oncomplete(move |event| {
            assert_eq!(event.rendered_buffer.length(), 555);
            complete_clone.store(true, Ordering::Relaxed);
        });

        let _ = executor::block_on(context.start_rendering());

        assert!(complete.load(Ordering::Relaxed));
    }

    fn require_send_sync<T: Send + Sync>(_: T) {}

    #[test]
    fn test_all_futures_thread_safe() {
        let context = OfflineAudioContext::new(2, 555, 44_100.);

        require_send_sync(context.start_rendering());
        require_send_sync(context.suspend(1.));
        require_send_sync(context.resume());
    }
}
