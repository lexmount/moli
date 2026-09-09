//! Communicates with the control thread and ships audio samples to the hardware

use std::any::Any;
use std::cell::Cell;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use dasp_sample::FromSample;
use futures_channel::{mpsc, oneshot};
use futures_util::StreamExt as _;

use super::AudioRenderQuantum;
use crate::buffer::AudioBuffer;
#[cfg(feature = "diagnostics")]
use crate::context::{AudioContextDiagnostics, AudioRenderThreadDiagnostics};
use crate::context::{
    AudioContextState, AudioNodeId, OfflineAudioContext, OfflineAudioContextCallback,
};
use crate::events::{EventDispatch, EventLoop};
use crate::message::ControlMessage;
use crate::node::ChannelInterpretation;
use crate::render::AudioWorkletGlobalScope;
use crate::stats::AudioStats;
use crate::RENDER_QUANTUM_SIZE;

use super::graph::Graph;

/// Operations running off the system-level audio callback
pub(crate) struct RenderThread {
    graph: Option<Graph>,
    sample_rate: f32,
    buffer_size: usize,
    /// number of channels of the backend stream, i.e. sound card number of
    /// channels clamped to MAX_CHANNELS
    number_of_channels: usize,
    suspended: bool,
    state: Arc<AtomicU8>,
    startup_pending: Option<Arc<AtomicBool>>,
    frames_played: Arc<AtomicU64>,
    receiver: Option<Receiver<ControlMessage>>,
    buffer_offset: Option<(usize, AudioRenderQuantum)>,
    stats: AudioStats,
    event_sender: Sender<EventDispatch>,
    garbage_collector: Option<llq::Producer<Box<dyn Any + Send>>>,
}

// SAFETY:
// The RenderThread is not Send/Sync since it contains `AudioRenderQuantum`s (which use Rc), but
// these are only accessed within the same thread (the render thread). Due to the cpal constraints
// we can neither move the RenderThread object into the render thread, nor can we initialize the
// Rc's in that thread.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for Graph {}
unsafe impl Sync for Graph {}
unsafe impl Send for RenderThread {}
unsafe impl Sync for RenderThread {}

impl std::fmt::Debug for RenderThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderThread")
            .field("sample_rate", &self.sample_rate)
            .field("buffer_size", &self.buffer_size)
            .field("frames_played", &self.frames_played.load(Ordering::Relaxed))
            .field("number_of_channels", &self.number_of_channels)
            .finish_non_exhaustive()
    }
}

impl RenderThread {
    #[cfg(feature = "diagnostics")]
    fn diagnostics(&self) -> AudioRenderThreadDiagnostics {
        AudioRenderThreadDiagnostics {
            sample_rate: self.sample_rate,
            buffer_size: self.buffer_size,
            frames_played: self.frames_played.load(Ordering::Relaxed),
            number_of_channels: self.number_of_channels,
            suspended: self.suspended,
        }
    }

    pub fn new(
        sample_rate: f32,
        number_of_channels: usize,
        receiver: Receiver<ControlMessage>,
        state: Arc<AtomicU8>,
        frames_played: Arc<AtomicU64>,
        stats: AudioStats,
        event_sender: Sender<EventDispatch>,
    ) -> Self {
        Self {
            graph: None,
            sample_rate,
            buffer_size: 0,
            number_of_channels,
            suspended: false,
            state,
            startup_pending: None,
            frames_played,
            receiver: Some(receiver),
            buffer_offset: None,
            stats,
            event_sender,
            garbage_collector: None,
        }
    }

    pub(crate) fn set_startup_pending(&mut self, startup_pending: Arc<AtomicBool>) {
        self.startup_pending = Some(startup_pending);
    }

    pub(crate) fn spawn_garbage_collector_thread(&mut self) {
        if self.garbage_collector.is_none() {
            let (gc_producer, gc_consumer) = llq::Queue::new().split();
            spawn_garbage_collector_thread(gc_consumer);
            self.garbage_collector = Some(gc_producer);
        }
    }

    #[inline(always)]
    fn handle_control_messages(&mut self) {
        if self.receiver.is_none() {
            return;
        }

        while let Ok(msg) = self.receiver.as_ref().unwrap().try_recv() {
            let result = self.handle_control_message(msg);
            if result.is_break() {
                return; // stop processing
            }
        }
    }

    fn handle_control_message(&mut self, msg: ControlMessage) -> ControlFlow<()> {
        use ControlMessage::*;

        match msg {
            RegisterNode {
                id: node_id,
                reclaim_id,
                node,
                inputs,
                outputs,
                channel_config,
            } => {
                self.graph.as_mut().unwrap().add_node(
                    node_id,
                    reclaim_id,
                    node,
                    inputs,
                    outputs,
                    channel_config,
                );
            }
            ConnectNode {
                from,
                to,
                output,
                input,
            } => {
                self.graph
                    .as_mut()
                    .unwrap()
                    .add_edge((from, output), (to, input));
            }
            DisconnectNode {
                from,
                output,
                to,
                input,
            } => {
                self.graph
                    .as_mut()
                    .unwrap()
                    .remove_edge((from, output), (to, input));
            }
            ControlHandleDropped { id } => {
                self.graph.as_mut().unwrap().mark_control_handle_dropped(id);
            }
            MarkCycleBreaker { id } => {
                self.graph.as_mut().unwrap().mark_cycle_breaker(id);
            }
            CloseAndRecycle { sender } => {
                self.set_state(AudioContextState::Suspended);
                let _ = sender.send(self.graph.take().unwrap());
                self.receiver = None;
                return ControlFlow::Break(()); // no further handling of ctrl msgs
            }
            Startup { graph } => {
                debug_assert!(self.graph.is_none());
                self.graph = Some(graph);
                if let Some(startup_pending) = self.startup_pending.as_ref() {
                    startup_pending.store(false, Ordering::Release);
                }
                self.set_state(AudioContextState::Running);
            }
            NodeMessage { id, mut msg } => {
                self.graph.as_mut().unwrap().route_message(id, msg.as_mut());
                if let Some(gc) = self.garbage_collector.as_mut() {
                    gc.push(msg)
                }
            }
            #[cfg(feature = "diagnostics")]
            RunDiagnostics { backend } => {
                let diagnostics = AudioContextDiagnostics {
                    backend,
                    render_thread: self.diagnostics(),
                    graph: self.graph.as_ref().unwrap().diagnostics(),
                };
                self.event_sender
                    .try_send(EventDispatch::diagnostics(diagnostics))
                    .expect("Unable to send diagnostics - channel is full");
            }
            Suspend { notify } => {
                self.suspended = true;
                self.set_state(AudioContextState::Suspended);
                notify.send();
            }
            Resume { notify } => {
                self.suspended = false;
                self.set_state(AudioContextState::Running);
                notify.send();
            }
            Close { notify } => {
                self.suspended = true;
                self.set_state(AudioContextState::Closed);
                notify.send();
            }

            SetChannelCount { id, count } => {
                self.graph.as_mut().unwrap().set_channel_count(id, count);
            }

            SetChannelCountMode { id, mode } => {
                self.graph
                    .as_mut()
                    .unwrap()
                    .set_channel_count_mode(id, mode);
            }

            SetChannelInterpretation { id, interpretation } => {
                self.graph
                    .as_mut()
                    .unwrap()
                    .set_channel_interpretation(id, interpretation);
            }
        }

        ControlFlow::Continue(()) // continue handling more messages
    }

    // Render method of the `OfflineAudioContext::start_rendering_sync`
    //
    // This method is not spec compliant and obviously marked as synchronous, so we
    // don't launch a thread.
    //
    // cf. https://webaudio.github.io/web-audio-api/#dom-offlineaudiocontext-startrendering
    pub fn render_audiobuffer_sync(
        mut self,
        context: &mut OfflineAudioContext,
        mut suspend_callbacks: Vec<(usize, Box<OfflineAudioContextCallback>)>,
        event_loop: &EventLoop,
    ) -> AudioBuffer {
        let length = context.length();
        let sample_rate = self.sample_rate;

        // construct a properly sized output buffer
        let mut buffer = Vec::with_capacity(self.number_of_channels);
        buffer.resize_with(buffer.capacity(), || Vec::with_capacity(length));

        let num_frames = length.div_ceil(RENDER_QUANTUM_SIZE);

        // Handle initial control messages
        self.handle_control_messages();

        for quantum in 0..num_frames {
            // Suspend at given times and run callbacks
            if suspend_callbacks.first().map(|&(q, _)| q) == Some(quantum) {
                let callback = suspend_callbacks.remove(0).1;
                (callback)(context);

                // Handle any control messages that may have been submitted by the callback
                self.handle_control_messages();
            }

            self.render_offline_quantum(&mut buffer);

            let events_were_handled = event_loop.handle_pending_events();
            if events_were_handled {
                // Handle any control messages that may have been submitted by the handler
                self.handle_control_messages();
            }
        }

        // call destructors of all alive nodes and handle any resulting events
        self.unload_graph();
        event_loop.handle_pending_events();

        AudioBuffer::from(buffer, sample_rate)
    }

    // Render method of the `OfflineAudioContext::start_rendering`
    //
    // This is the async interface, as compared to render_audiobuffer_sync
    //
    // cf. https://webaudio.github.io/web-audio-api/#dom-offlineaudiocontext-startrendering
    pub async fn render_audiobuffer(
        mut self,
        length: usize,
        mut suspend_callbacks: Vec<(usize, oneshot::Sender<()>)>,
        mut resume_receiver: mpsc::Receiver<()>,
        event_loop: &EventLoop,
    ) -> AudioBuffer {
        let sample_rate = self.sample_rate;

        // construct a properly sized output buffer
        let mut buffer = Vec::with_capacity(self.number_of_channels);
        buffer.resize_with(buffer.capacity(), || Vec::with_capacity(length));

        let num_frames = length.div_ceil(RENDER_QUANTUM_SIZE);

        // Handle addition/removal of nodes/edges
        self.handle_control_messages();

        for quantum in 0..num_frames {
            // Suspend at given times and run callbacks
            if suspend_callbacks.first().map(|&(q, _)| q) == Some(quantum) {
                let sender = suspend_callbacks.remove(0).1;
                sender.send(()).unwrap();
                resume_receiver.next().await;

                // Handle addition/removal of nodes/edges
                self.handle_control_messages();
            }

            self.render_offline_quantum(&mut buffer);

            let events_were_handled = event_loop.handle_pending_events();
            if events_were_handled {
                // Handle any control messages that may have been submitted by the handler
                self.handle_control_messages();
            }
        }

        // call destructors of all alive nodes and handle any resulting events
        self.unload_graph();
        event_loop.handle_pending_events();

        AudioBuffer::from(buffer, sample_rate)
    }

    /// Render a single quantum into an AudioBuffer
    fn render_offline_quantum(&mut self, buffer: &mut [Vec<f32>]) {
        // Update time
        let current_frame = self
            .frames_played
            .fetch_add(RENDER_QUANTUM_SIZE as u64, Ordering::Relaxed);
        let current_time = current_frame as f64 / self.sample_rate as f64;

        let scope = AudioWorkletGlobalScope {
            current_frame,
            current_time,
            sample_rate: self.sample_rate,
            event_sender: self.event_sender.clone(),
            node_id: Cell::new(AudioNodeId(0)), // placeholder value
        };

        // Render audio graph
        let graph = self.graph.as_mut().unwrap();

        // For x64 and aarch, process with denormal floats disabled (for performance, #194)
        #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
        let rendered = unsafe {
            // SAFETY: potentially risky - "modifying the masking flags, rounding mode, or
            // denormals-are-zero mode flags leads to immediate Undefined Behavior: Rust assumes
            // that these are always in their default state and will optimize accordingly."
            no_denormals::no_denormals(|| graph.render(&scope))
        };
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
        let rendered = graph.render(&scope);

        // Use a specialized copyToChannel implementation for performance
        let remaining = (buffer[0].capacity() - buffer[0].len()).min(RENDER_QUANTUM_SIZE);
        let channels = rendered.channels();
        buffer.iter_mut().enumerate().for_each(|(i, b)| {
            let c = channels
                .get(i)
                .map(AsRef::as_ref)
                // When there are no input nodes for the destination, only a single silent channel
                // is emitted. So manually pad the missing channels with silence
                .unwrap_or(&[0.; RENDER_QUANTUM_SIZE]);
            b.extend_from_slice(&c[..remaining]);
        });
    }

    /// Run destructors of all alive nodes in the audio graph
    fn unload_graph(mut self) {
        let current_frame = self.frames_played.load(Ordering::Relaxed);
        let current_time = current_frame as f64 / self.sample_rate as f64;

        let scope = AudioWorkletGlobalScope {
            current_frame,
            current_time,
            sample_rate: self.sample_rate,
            event_sender: self.event_sender.clone(),
            node_id: Cell::new(AudioNodeId(0)), // placeholder value
        };
        self.graph.take().unwrap().before_drop(&scope);
    }

    pub fn render<S: FromSample<f32> + Clone>(&mut self, output_buffer: &mut [S]) {
        // Collect timing information
        let render_start = Instant::now();
        let frames = output_buffer.len() / self.number_of_channels;

        // Perform actual rendering

        // For x64 and aarch, process with denormal floats disabled (for performance, #194)
        #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
        unsafe {
            // SAFETY: potentially risky - "modifying the masking flags, rounding mode, or
            // denormals-are-zero mode flags leads to immediate Undefined Behavior: Rust assumes
            // that these are always in their default state and will optimize accordingly."
            no_denormals::no_denormals(|| self.render_inner(output_buffer))
        };
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
        self.render_inner(output_buffer);

        let render_duration_ns = render_start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        let callback_budget_ns = if self.sample_rate <= 0. {
            0
        } else {
            ((frames as f64 / self.sample_rate as f64) * 1_000_000_000.).round() as u64
        };
        self.stats
            .record_render_callback(render_duration_ns, callback_budget_ns);
    }

    fn render_inner<S: FromSample<f32> + Clone>(&mut self, mut output_buffer: &mut [S]) {
        self.buffer_size = output_buffer.len();

        // There may be audio frames left over from the previous render call,
        // if the cpal buffer size did not align with our internal RENDER_QUANTUM_SIZE
        if let Some((offset, prev_rendered)) = self.buffer_offset.take() {
            let leftover_len = (RENDER_QUANTUM_SIZE - offset) * self.number_of_channels;
            // split the leftover frames slice, to fit in `buffer`
            let (first, next) = output_buffer.split_at_mut(leftover_len.min(output_buffer.len()));

            // copy rendered audio into output slice
            for i in 0..self.number_of_channels {
                let output = first.iter_mut().skip(i).step_by(self.number_of_channels);
                let channel = prev_rendered.channel_data(i)[offset..].iter();
                for (sample, input) in output.zip(channel) {
                    let value = S::from_sample_(*input);
                    *sample = value;
                }
            }

            // exit early if we are done filling the buffer with the previously rendered data
            if next.is_empty() {
                self.buffer_offset = Some((
                    offset + first.len() / self.number_of_channels,
                    prev_rendered,
                ));
                return;
            }

            // if there's still space left in the buffer, continue rendering
            output_buffer = next;
        }

        // handle addition/removal of nodes/edges
        self.handle_control_messages();

        // if the thread is still booting, suspended, or shutting down, fill with silence
        if self.suspended || !self.graph.as_ref().is_some_and(Graph::is_active) {
            output_buffer.fill(S::from_sample_(0.));
            return;
        }

        // The audio graph is rendered in chunks of RENDER_QUANTUM_SIZE frames.  But some audio backends
        // may not be able to emit chunks of this size.
        let chunk_size = RENDER_QUANTUM_SIZE * self.number_of_channels;

        for data in output_buffer.chunks_mut(chunk_size) {
            // update time
            let current_frame = self
                .frames_played
                .fetch_add(RENDER_QUANTUM_SIZE as u64, Ordering::Relaxed);
            let current_time = current_frame as f64 / self.sample_rate as f64;

            let scope = AudioWorkletGlobalScope {
                current_frame,
                current_time,
                sample_rate: self.sample_rate,
                event_sender: self.event_sender.clone(),
                node_id: Cell::new(AudioNodeId(0)), // placeholder value
            };

            // render audio graph, clone it in case we need to mutate/store the value later
            let mut destination_buffer = self.graph.as_mut().unwrap().render(&scope).clone();

            // online AudioContext allows channel count to be less than the number
            // of channels of the backend stream, i.e. number of channels of the
            // soundcard clamped to MAX_CHANNELS.
            if destination_buffer.number_of_channels() < self.number_of_channels {
                destination_buffer.mix(self.number_of_channels, ChannelInterpretation::Discrete);
            }

            // copy rendered audio into output slice
            for i in 0..self.number_of_channels {
                let output = data.iter_mut().skip(i).step_by(self.number_of_channels);
                let channel = destination_buffer.channel_data(i).iter();
                for (sample, input) in output.zip(channel) {
                    let value = S::from_sample_(*input);
                    *sample = value;
                }
            }

            if data.len() != chunk_size {
                // this is the last chunk, and it contained less than RENDER_QUANTUM_SIZE samples
                let channel_offset = data.len() / self.number_of_channels;
                debug_assert!(channel_offset < RENDER_QUANTUM_SIZE);
                self.buffer_offset = Some((channel_offset, destination_buffer));
            }

            // handle addition/removal of nodes/edges
            self.handle_control_messages();
        }
    }

    fn set_state(&self, state: AudioContextState) {
        self.state.store(state as u8, Ordering::Relaxed);
        self.event_sender
            .try_send(EventDispatch::state_change(state))
            .ok();
    }
}

impl Drop for RenderThread {
    fn drop(&mut self) {
        if let Some(gc) = self.garbage_collector.as_mut() {
            gc.push(llq::Node::new(Box::new(TerminateGarbageCollectorThread)))
        }
        log::info!("Audio render thread has been dropped");
    }
}

// Controls the polling frequency of the garbage collector thread.
const GARBAGE_COLLECTOR_THREAD_TIMEOUT: Duration = Duration::from_millis(100);

// Poison pill that terminates the garbage collector thread.
#[derive(Debug)]
struct TerminateGarbageCollectorThread;

// Spawns a sidecar thread of the `RenderThread` for dropping resources.
fn spawn_garbage_collector_thread(consumer: llq::Consumer<Box<dyn Any + Send>>) {
    let _join_handle = std::thread::spawn(move || run_garbage_collector_thread(consumer));
}

fn run_garbage_collector_thread(mut consumer: llq::Consumer<Box<dyn Any + Send>>) {
    log::info!("Entering garbage collector thread");
    loop {
        if let Some(node) = consumer.pop() {
            if node
                .as_ref()
                .downcast_ref::<TerminateGarbageCollectorThread>()
                .is_some()
            {
                log::info!("Terminating garbage collector thread");
                break;
            }
            // Implicitly drop the received node.
        } else {
            std::thread::sleep(GARBAGE_COLLECTOR_THREAD_TIMEOUT);
        }
    }
    log::info!("Exiting garbage collector thread");
}
