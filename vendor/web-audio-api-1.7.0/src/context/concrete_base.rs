//! The `ConcreteBaseAudioContext` type

use crate::context::{
    AudioContextRegistration, AudioContextState, AudioNodeId, BaseAudioContext,
    DESTINATION_NODE_ID, LISTENER_NODE_ID, LISTENER_PARAM_IDS,
};
use crate::events::{EventDispatch, EventHandler, EventLoop, EventType};
use crate::message::ControlMessage;
use crate::node::{AudioDestinationNode, AudioNode, AudioNodeOptions, ChannelConfig};
use crate::param::AudioParam;
use crate::render::AudioProcessor;
use crate::spatial::AudioListenerParams;

use crate::AudioListener;

use crossbeam_channel::{SendError, Sender};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};

/// This struct assigns new [`AudioNodeId`]s for [`AudioNode`]s
///
/// It reuses the ids of decommissioned nodes to prevent unbounded growth of the audio graphs node
/// list (which is stored in a Vec indexed by the AudioNodeId).
struct AudioNodeIdProvider {
    /// incrementing id
    id_inc: AtomicU64,
    /// receiver for decommissioned AudioNodeIds, which can be reused
    id_consumer: Mutex<llq::Consumer<AudioNodeId>>,
}

impl AudioNodeIdProvider {
    fn new(id_consumer: llq::Consumer<AudioNodeId>) -> Self {
        Self {
            id_inc: AtomicU64::new(0),
            id_consumer: Mutex::new(id_consumer),
        }
    }

    fn get(&self) -> AudioNodeId {
        if let Some(available_id) = self.id_consumer.lock().unwrap().pop() {
            llq::Node::into_inner(available_id)
        } else {
            AudioNodeId(self.id_inc.fetch_add(1, Ordering::Relaxed))
        }
    }
}

/// The struct that corresponds to the Javascript `BaseAudioContext` object.
///
/// This object is returned from the `base()` method on
/// [`AudioContext`](crate::context::AudioContext) and
/// [`OfflineAudioContext`](crate::context::OfflineAudioContext), and the `context()` method on
/// `AudioNode`s.
///
/// The `ConcreteBaseAudioContext` allows for shallow cloning (using an `Arc` internally).
#[allow(clippy::module_name_repetitions)]
#[derive(Clone)]
#[doc(hidden)]
pub struct ConcreteBaseAudioContext {
    inner: Arc<ConcreteBaseAudioContextInner>,
}

impl PartialEq for ConcreteBaseAudioContext {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl std::fmt::Debug for ConcreteBaseAudioContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BaseAudioContext")
            .field("id", &self.address())
            .field("state", &self.state())
            .field("sample_rate", &self.sample_rate())
            .field("current_time", &self.current_time())
            .field("max_channel_count", &self.max_channel_count())
            .field("offline", &self.offline())
            .finish_non_exhaustive()
    }
}

/// Inner representation of the `ConcreteBaseAudioContext`
///
/// These fields are wrapped inside an `Arc` in the actual `ConcreteBaseAudioContext`.
struct ConcreteBaseAudioContextInner {
    /// sample rate in Hertz
    sample_rate: f32,
    /// max number of speaker output channels
    max_channel_count: usize,
    /// provider for new AudioNodeIds
    audio_node_id_provider: AudioNodeIdProvider,
    /// destination node's current channel count
    destination_channel_config: ChannelConfig,
    /// message channel from control to render thread
    render_channel: RwLock<Sender<ControlMessage>>,
    /// control messages staged while the render thread is suspended
    suspended_messages: Mutex<Option<Vec<ControlMessage>>>,
    /// control messages that cannot be sent immediately
    queued_messages: Mutex<Vec<ControlMessage>>,
    /// number of frames played
    frames_played: Arc<AtomicU64>,
    /// control msg to add the AudioListener, to be sent when the first panner is created
    queued_audio_listener_msgs: Mutex<Vec<ControlMessage>>,
    /// AudioListener fields
    listener_params: Option<AudioListenerParams>,
    /// Denotes if this AudioContext is offline or not
    offline: bool,
    /// Current state of the `ConcreteBaseAudioContext`, shared with the RenderThread
    state: Arc<AtomicU8>,
    /// Stores the event handlers
    event_loop: EventLoop,
    /// Sender for events that will be handled by the EventLoop
    event_send: Sender<EventDispatch>,
    /// Current audio graph connections (from node, output port, to node, input port)
    connections: Mutex<HashSet<(AudioNodeId, usize, AudioNodeId, usize)>>,
}

impl BaseAudioContext for ConcreteBaseAudioContext {
    fn base(&self) -> &ConcreteBaseAudioContext {
        self
    }
}

impl ConcreteBaseAudioContext {
    /// Creates a `BaseAudioContext` instance
    #[allow(clippy::too_many_arguments)] // TODO refactor with builder pattern
    pub(super) fn new(
        sample_rate: f32,
        max_channel_count: usize,
        state: Arc<AtomicU8>,
        frames_played: Arc<AtomicU64>,
        render_channel: Sender<ControlMessage>,
        event_send: Sender<EventDispatch>,
        event_loop: EventLoop,
        offline: bool,
        node_id_consumer: llq::Consumer<AudioNodeId>,
    ) -> Self {
        let audio_node_id_provider = AudioNodeIdProvider::new(node_id_consumer);

        let base_inner = ConcreteBaseAudioContextInner {
            sample_rate,
            max_channel_count,
            render_channel: RwLock::new(render_channel),
            suspended_messages: Mutex::new(None),
            queued_messages: Mutex::new(Vec::new()),
            audio_node_id_provider,
            destination_channel_config: AudioNodeOptions::default().into(),
            frames_played,
            queued_audio_listener_msgs: Mutex::new(Vec::new()),
            listener_params: None,
            offline,
            state,
            event_loop,
            event_send,
            connections: Mutex::new(HashSet::new()),
        };
        let base = Self {
            inner: Arc::new(base_inner),
        };

        // Online AudioContext should start with stereo channels by default
        let initial_channel_count = if offline {
            max_channel_count
        } else {
            2.min(max_channel_count)
        };

        let (listener_params, destination_channel_config) = {
            // Register magical nodes. We should not store the nodes inside our context since that
            // will create a cyclic reference, but we can reconstruct a new instance on the fly
            // when requested
            let dest = AudioDestinationNode::new(&base, initial_channel_count);
            let destination_channel_config = dest.into_channel_config();
            let listener = crate::spatial::AudioListenerNode::new(&base);

            let listener_params = listener.into_fields();
            let AudioListener {
                position_x,
                position_y,
                position_z,
                forward_x,
                forward_y,
                forward_z,
                up_x,
                up_y,
                up_z,
            } = listener_params;

            let listener_params = AudioListenerParams {
                position_x: position_x.into_raw_parts(),
                position_y: position_y.into_raw_parts(),
                position_z: position_z.into_raw_parts(),
                forward_x: forward_x.into_raw_parts(),
                forward_y: forward_y.into_raw_parts(),
                forward_z: forward_z.into_raw_parts(),
                up_x: up_x.into_raw_parts(),
                up_y: up_y.into_raw_parts(),
                up_z: up_z.into_raw_parts(),
            };

            (listener_params, destination_channel_config)
        }; // Nodes will drop now, so base.inner has no copies anymore

        let mut base = base;
        let inner_mut = Arc::get_mut(&mut base.inner).unwrap();
        inner_mut.listener_params = Some(listener_params);
        inner_mut.destination_channel_config = destination_channel_config;

        // Validate if the hardcoded node IDs line up
        debug_assert_eq!(
            base.inner
                .audio_node_id_provider
                .id_inc
                .load(Ordering::Relaxed),
            LISTENER_PARAM_IDS.end,
        );

        // HRTF is loaded on the control thread when a PannerNode selects that
        // model. Do not resample its entire database for contexts that never
        // create a spatial panner (including headless oscillator/analyser use).

        base
    }

    pub(crate) fn address(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }

    /// Construct a new pair of [`AudioNode`] and [`AudioProcessor`]
    pub(crate) fn register<
        T: AudioNode,
        F: FnOnce(AudioContextRegistration) -> (T, Box<dyn AudioProcessor>),
    >(
        &self,
        f: F,
    ) -> T {
        // create a unique id for this node
        let id = self.inner.audio_node_id_provider.get();
        let registration = AudioContextRegistration {
            id,
            context: self.clone(),
        };

        // create the node and its renderer
        let (node, render) = (f)(registration);

        // pass the renderer to the audio graph
        let message = ControlMessage::RegisterNode {
            id,
            reclaim_id: llq::Node::new(id),
            node: render,
            inputs: node.number_of_inputs(),
            outputs: node.number_of_outputs(),
            channel_config: node.channel_config().inner(),
        };

        // if this is the AudioListener or its params, do not add it to the graph just yet
        if id == LISTENER_NODE_ID || LISTENER_PARAM_IDS.contains(&id.0) {
            let mut queued_audio_listener_msgs =
                self.inner.queued_audio_listener_msgs.lock().unwrap();
            queued_audio_listener_msgs.push(message);
        } else {
            self.send_control_msg(message);
            self.resolve_queued_control_msgs(id);
        }

        node
    }

    /// Send a control message to the render thread
    ///
    /// When the render thread is closed or crashed, the message is discarded and a log warning is
    /// emitted.
    pub(crate) fn send_control_msg(&self, msg: ControlMessage) {
        if self.state() != AudioContextState::Closed {
            let sender = self.inner.render_channel.read().unwrap();
            // if the context is suspended, buffer the message and don't send it
            if let Some(queued) = self.inner.suspended_messages.lock().unwrap().as_mut() {
                queued.push(msg);
                return;
            }

            let result = sender.send(msg);
            if result.is_err() {
                log::warn!("Discarding control message - render thread is closed");
            }
        }
    }

    pub(crate) fn suspend_control_msgs(&self, msg: ControlMessage) {
        let sender = self.inner.render_channel.read().unwrap();
        *self.inner.suspended_messages.lock().unwrap() = Some(Vec::new());
        if sender.send(msg).is_err() {
            log::warn!("Discarding control message - render thread is closed");
        }
    }

    pub(crate) fn resume_control_msgs(&self, msg: ControlMessage) {
        let sender = self.inner.render_channel.read().unwrap();
        let messages = self
            .inner
            .suspended_messages
            .lock()
            .unwrap()
            .take()
            .unwrap_or_default();

        for msg in messages {
            if sender.send(msg).is_err() {
                log::warn!("Discarding control message - render thread is closed");
                return;
            }
        }

        if sender.send(msg).is_err() {
            log::warn!("Discarding control message - render thread is closed");
        }
    }

    pub(crate) fn send_event(&self, msg: EventDispatch) -> Result<(), SendError<EventDispatch>> {
        self.inner.event_send.send(msg)
    }

    pub(crate) fn lock_control_msg_sender(&self) -> RwLockWriteGuard<'_, Sender<ControlMessage>> {
        self.inner.render_channel.write().unwrap()
    }

    pub(super) fn mark_node_dropped(&self, id: AudioNodeId) {
        // Ignore magic nodes
        if id == DESTINATION_NODE_ID || id == LISTENER_NODE_ID || LISTENER_PARAM_IDS.contains(&id.0)
        {
            return;
        }

        // Inform render thread that the control thread AudioNode no longer has any handles
        let message = ControlMessage::ControlHandleDropped { id };
        self.send_control_msg(message);

        // Clear the connection administration for this node, the node id may be recycled later
        self.inner
            .connections
            .lock()
            .unwrap()
            .retain(|&(from, _output, to, _input)| from != id && to != id);
    }

    /// Inform render thread that this node can act as a cycle breaker
    #[doc(hidden)]
    pub fn mark_cycle_breaker(&self, reg: &AudioContextRegistration) {
        let id = reg.id();
        let message = ControlMessage::MarkCycleBreaker { id };
        self.send_control_msg(message);
    }

    /// `ChannelConfig` of the `AudioDestinationNode`
    pub(super) fn destination_channel_config(&self) -> ChannelConfig {
        self.inner.destination_channel_config.clone()
    }

    /// Returns the `AudioListener` which is used for 3D spatialization
    pub(super) fn listener(&self) -> AudioListener {
        // instruct to BaseContext to add the AudioListener if it has not already
        self.base().ensure_audio_listener_present();

        let mut ids = LISTENER_PARAM_IDS.map(|i| AudioContextRegistration {
            id: AudioNodeId(i),
            context: self.clone(),
        });
        let params = self.inner.listener_params.as_ref().unwrap();

        AudioListener {
            position_x: AudioParam::from_raw_parts(ids.next().unwrap(), params.position_x.clone()),
            position_y: AudioParam::from_raw_parts(ids.next().unwrap(), params.position_y.clone()),
            position_z: AudioParam::from_raw_parts(ids.next().unwrap(), params.position_z.clone()),
            forward_x: AudioParam::from_raw_parts(ids.next().unwrap(), params.forward_x.clone()),
            forward_y: AudioParam::from_raw_parts(ids.next().unwrap(), params.forward_y.clone()),
            forward_z: AudioParam::from_raw_parts(ids.next().unwrap(), params.forward_z.clone()),
            up_x: AudioParam::from_raw_parts(ids.next().unwrap(), params.up_x.clone()),
            up_y: AudioParam::from_raw_parts(ids.next().unwrap(), params.up_y.clone()),
            up_z: AudioParam::from_raw_parts(ids.next().unwrap(), params.up_z.clone()),
        }
    }

    /// Returns state of current context
    #[must_use]
    pub(super) fn state(&self) -> AudioContextState {
        self.inner.state.load(Ordering::Acquire).into()
    }

    /// Updates state of current context
    pub(super) fn set_state(&self, state: AudioContextState) {
        // Only used from OfflineAudioContext or suspended AudioContext, otherwise the state
        // changed are spawned from the render thread
        let current_state = self.state();
        if current_state != state {
            self.inner.state.store(state as u8, Ordering::Release);
            let _ = self.send_event(EventDispatch::state_change(state));
        }
    }

    /// The sample rate (in sample-frames per second) at which the `AudioContext` handles audio.
    #[must_use]
    pub(super) fn sample_rate(&self) -> f32 {
        self.inner.sample_rate
    }

    /// This is the time in seconds of the sample frame immediately following the last sample-frame
    /// in the block of audio most recently processed by the context’s rendering graph.
    #[must_use]
    // web audio api specification requires that `current_time` returns an f64
    // std::sync::AtomicsF64 is not currently implemented in the standard library
    // Currently, we have no other choice than casting an u64 into f64, with possible loss of precision
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn current_time(&self) -> f64 {
        self.inner.frames_played.load(Ordering::SeqCst) as f64 / self.inner.sample_rate as f64
    }

    /// Maximum available channels for the audio destination
    #[must_use]
    pub(crate) fn max_channel_count(&self) -> usize {
        self.inner.max_channel_count
    }

    /// Release queued control messages to the render thread that were blocking on the availability
    /// of the Node with the given `id`
    fn resolve_queued_control_msgs(&self, id: AudioNodeId) {
        // resolve control messages that depend on this registration
        let mut queued = self.inner.queued_messages.lock().unwrap();
        let mut i = 0; // waiting for Vec::drain_filter to stabilize
        while i < queued.len() {
            if matches!(&queued[i], ControlMessage::ConnectNode {to, ..} if *to == id) {
                let m = queued.remove(i);
                self.send_control_msg(m);
            } else {
                i += 1;
            }
        }
    }

    /// Connects the output of the `from` audio node to the input of the `to` audio node
    pub(crate) fn connect(&self, from: AudioNodeId, to: AudioNodeId, output: usize, input: usize) {
        let inserted = self
            .inner
            .connections
            .lock()
            .unwrap()
            .insert((from, output, to, input));

        if !inserted {
            return; // do not allow duplicated edges
        }

        let message = ControlMessage::ConnectNode {
            from,
            to,
            output,
            input,
        };
        self.send_control_msg(message);
    }

    /// Schedule a connection of an `AudioParam` to the `AudioNode` it belongs to
    ///
    /// It is not performed immediately as the `AudioNode` is not registered at this point.
    pub(super) fn queue_audio_param_connect(&self, param: &AudioParam, audio_node: AudioNodeId) {
        // no need to store these type of connections in self.inner.connections

        let message = ControlMessage::ConnectNode {
            from: param.registration().id(),
            to: audio_node,
            output: 0,
            input: usize::MAX, // audio params connect to the 'hidden' input port
        };
        self.inner.queued_messages.lock().unwrap().push(message);
    }

    /// Disconnects outputs of the audio node, possibly filtered by output node, input, output.
    pub(crate) fn disconnect(
        &self,
        from: AudioNodeId,
        output: Option<usize>,
        to: Option<AudioNodeId>,
        input: Option<usize>,
    ) {
        // check if the node was connected, otherwise panic
        let mut has_disconnected = false;
        let mut connections = self.inner.connections.lock().unwrap();
        connections.retain(|&(c_from, c_output, c_to, c_input)| {
            let retain = c_from != from
                || c_output != output.unwrap_or(c_output)
                || c_to != to.unwrap_or(c_to)
                || c_input != input.unwrap_or(c_input);
            if !retain {
                has_disconnected = true;
                let message = ControlMessage::DisconnectNode {
                    from,
                    to: c_to,
                    input: c_input,
                    output: c_output,
                };
                self.send_control_msg(message);
            }
            retain
        });

        // make sure to drop the MutexGuard before the panic to avoid poisoning
        drop(connections);

        if !has_disconnected && to.is_some() {
            panic!("InvalidAccessError - attempting to disconnect unconnected nodes");
        }
    }

    /// Connect the `AudioListener` to a `PannerNode`
    pub(crate) fn connect_listener_to_panner(&self, panner: AudioNodeId) {
        self.connect(LISTENER_NODE_ID, panner, 0, usize::MAX);
    }

    /// Add the [`AudioListener`] to the audio graph (if not already)
    pub(crate) fn ensure_audio_listener_present(&self) {
        let mut queued_audio_listener_msgs = self.inner.queued_audio_listener_msgs.lock().unwrap();
        let mut released = false;
        while let Some(message) = queued_audio_listener_msgs.pop() {
            // add the AudioListenerRenderer to the graph
            self.send_control_msg(message);
            released = true;
        }

        if released {
            // connect the AudioParamRenderers to the Listener
            self.resolve_queued_control_msgs(LISTENER_NODE_ID);

            // hack: Connect the listener to the destination node to force it to render at each
            // quantum. Abuse the magical usize::MAX port so it acts as an AudioParam and has no side
            // effects
            self.connect(LISTENER_NODE_ID, DESTINATION_NODE_ID, 0, usize::MAX);
        }
    }

    /// Returns true if this is `OfflineAudioContext` (false when it is an `AudioContext`)
    pub(crate) fn offline(&self) -> bool {
        self.inner.offline
    }

    pub(crate) fn set_event_handler(&self, event: EventType, callback: EventHandler) {
        self.inner.event_loop.set_handler(event, callback);
    }

    pub(crate) fn clear_event_handler(&self, event: EventType) {
        self.inner.event_loop.clear_handler(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::OfflineAudioContext;

    #[test]
    fn test_provide_node_id() {
        let (mut id_producer, id_consumer) = llq::Queue::new().split();
        let provider = AudioNodeIdProvider::new(id_consumer);
        assert_eq!(provider.get().0, 0); // newly assigned
        assert_eq!(provider.get().0, 1); // newly assigned
        id_producer.push(llq::Node::new(AudioNodeId(0)));
        assert_eq!(provider.get().0, 0); // reused
        assert_eq!(provider.get().0, 2); // newly assigned
    }

    #[test]
    fn test_connect_disconnect() {
        let context = OfflineAudioContext::new(1, 128, 48000.);
        let node1 = context.create_constant_source();
        let node2 = context.create_gain();

        // connection list starts empty
        assert!(context.base().inner.connections.lock().unwrap().is_empty());

        node1.disconnect(); // never panic for plain disconnect calls

        node1.connect(&node2);

        // connection should be registered
        assert_eq!(context.base().inner.connections.lock().unwrap().len(), 1);

        node1.disconnect();
        assert!(context.base().inner.connections.lock().unwrap().is_empty());

        node1.connect(&node2);
        assert_eq!(context.base().inner.connections.lock().unwrap().len(), 1);

        node1.disconnect_dest(&node2);
        assert!(context.base().inner.connections.lock().unwrap().is_empty());
    }

    #[test]
    #[should_panic]
    fn test_disconnect_not_existing() {
        let context = OfflineAudioContext::new(1, 128, 48000.);
        let node1 = context.create_constant_source();
        let node2 = context.create_gain();

        node1.disconnect_dest(&node2);
    }

    #[test]
    fn test_mark_node_dropped() {
        let context = OfflineAudioContext::new(1, 128, 48000.);

        let node1 = context.create_constant_source();
        let node2 = context.create_gain();

        node1.connect(&node2);
        context.base().mark_node_dropped(node1.registration().id());

        // dropping should clear connections administration
        assert!(context.base().inner.connections.lock().unwrap().is_empty());
    }
}
