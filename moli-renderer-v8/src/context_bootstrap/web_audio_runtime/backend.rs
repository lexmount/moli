//! Native Web Audio state. The JS wrapper, not a page-global strong root, owns
//! its lifetime. GC reclaims abandoned nodes/contexts; isolate drop also drains
//! the store. No raw pointers or JavaScript-visible state IDs are exposed.

use crate::util::{get_private_value, set_private_value};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};
use web_audio_api::context::{
    AudioContext, AudioContextOptions, BaseAudioContext, OfflineAudioContext,
};
use web_audio_api::node::{AnalyserNode, AudioDestinationNode, AudioNode, BiquadFilterNode};
use web_audio_api::worklet::{
    AudioParamValues, AudioWorkletGlobalScope, AudioWorkletNode, AudioWorkletNodeOptions,
    AudioWorkletProcessor,
};
use web_audio_api::{AudioParam, AudioParamDescriptor, AutomationRate};

const SLOT: &str = "__moliWebAudioBackend";
type Store = Rc<RefCell<States>>;
pub(super) type StateRef = Rc<RefCell<State>>;

#[derive(Default)]
struct States {
    next_id: u64,
    entries: HashMap<u64, (v8::Weak<v8::Object>, StateRef)>,
}

pub(super) enum State {
    Context(Context),
    Node(Node),
    Param(Parameter),
    // Existing JS worklets load modules and exchange messages, but do not yet
    // run process() on the audio thread. Preserve their control graph without
    // inventing a silent or pass-through native processor.
    ModuleWorklet,
}

/// The nominal range can depend on the context's sample rate, whereas an
/// AudioWorklet's descriptors are static. Keep that immutable range alongside
/// its actual timeline; the oscillator also applies it at a-rate on the DSP side.
#[derive(Clone)]
pub(super) struct Parameter {
    native: AudioParam,
    minimum: f32,
    maximum: f32,
    fixed_rate: Option<AutomationRate>,
}

impl Parameter {
    pub(super) fn new(native: AudioParam) -> Self {
        let minimum = native.min_value();
        let maximum = native.max_value();
        Self {
            native,
            minimum,
            maximum,
            fixed_rate: None,
        }
    }
    pub(super) fn with_range(mut self, minimum: f32, maximum: f32) -> Self {
        self.minimum = minimum;
        self.maximum = maximum;
        self
    }
    pub(super) fn automation_rate(&self) -> AutomationRate {
        self.fixed_rate
            .unwrap_or_else(|| self.native.automation_rate())
    }
    pub(super) fn set_automation_rate(&self, rate: AutomationRate) -> bool {
        if self.fixed_rate.is_some() {
            return false;
        }
        self.native.set_automation_rate(rate);
        true
    }
    pub(super) fn value(&self) -> f32 {
        self.native.value().clamp(self.minimum, self.maximum)
    }
    pub(super) fn default_value(&self) -> f32 {
        self.native.default_value()
    }
    pub(super) fn min_value(&self) -> f32 {
        self.minimum
    }
    pub(super) fn max_value(&self) -> f32 {
        self.maximum
    }
    pub(super) fn set_value(&self, value: f32) {
        self.native.set_value(value);
    }
    pub(super) fn set_value_at_time(&self, value: f32, when: f64) {
        self.native.set_value_at_time(value, when);
    }
}

pub(super) enum Context {
    Realtime(Box<AudioContext>),
    Offline {
        context: Box<OfflineAudioContext>,
        rendered: bool,
    },
}

impl Drop for Context {
    fn drop(&mut self) {
        if let Self::Realtime(context) = self {
            // The upstream AudioContext intentionally keeps playback alive on
            // drop. A collected wrapper or disposed document must instead stop
            // its headless audio thread, including contexts never explicitly
            // closed by page code.
            context.close_sync();
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum NodeKind {
    Destination,
    Oscillator,
    Compressor,
    Biquad,
    Analyser,
}

pub(super) enum Node {
    Destination(AudioDestinationNode),
    Oscillator(super::oscillator_dsp::Oscillator),
    Compressor {
        node: AudioWorkletNode,
        reduction: Arc<AtomicU32>,
        demanded: Arc<AtomicBool>,
    },
    Biquad(BiquadFilterNode),
    Analyser(Box<AnalyserNode>),
}

impl Context {
    pub(super) fn realtime() -> Self {
        // Headless processing must not open the host's speakers or microphone.
        Self::Realtime(Box::new(AudioContext::new(AudioContextOptions {
            sample_rate: Some(44_100.0),
            sink_id: "none".into(),
            ..Default::default()
        })))
    }

    pub(super) fn offline(channels: usize, length: usize, sample_rate: f32) -> Self {
        Self::Offline {
            context: Box::new(OfflineAudioContext::new(channels, length, sample_rate)),
            rendered: false,
        }
    }

    pub(super) fn create_node(&self, kind: NodeKind) -> Node {
        match self {
            Self::Realtime(context) => Node::create(context.as_ref(), kind),
            Self::Offline { context, .. } => Node::create(context.as_ref(), kind),
        }
    }

    pub(super) fn current_time(&self) -> f64 {
        match self {
            Self::Realtime(context) => context.current_time(),
            Self::Offline { context, .. } => context.current_time(),
        }
    }
}

impl Node {
    fn create<C: BaseAudioContext>(context: &C, kind: NodeKind) -> Self {
        match kind {
            NodeKind::Destination => Self::Destination(context.destination()),
            NodeKind::Oscillator => {
                Self::Oscillator(super::oscillator_dsp::Oscillator::new(context))
            }
            NodeKind::Biquad => Self::Biquad(context.create_biquad_filter()),
            NodeKind::Analyser => Self::Analyser(Box::new(context.create_analyser())),
            NodeKind::Compressor => {
                let reduction = Arc::new(AtomicU32::new(0));
                let demanded = Arc::new(AtomicBool::new(false));
                let node = AudioWorkletNode::new::<CompressorProcessor>(
                    context,
                    AudioWorkletNodeOptions {
                        processor_options: (
                            context.sample_rate(),
                            Arc::clone(&reduction),
                            Arc::clone(&demanded),
                        ),
                        output_channel_count: vec![2],
                        ..Default::default()
                    },
                );
                Self::Compressor {
                    node,
                    reduction,
                    demanded,
                }
            }
        }
    }

    pub(super) fn audio_node(&self) -> &dyn AudioNode {
        match self {
            Self::Destination(node) => node,
            Self::Oscillator(node) => &node.node,
            Self::Compressor { node, .. } => node,
            Self::Biquad(node) => node,
            Self::Analyser(node) => node.as_ref(),
        }
    }

    pub(super) fn param(&self, name: &str) -> Parameter {
        let native = match self {
            Self::Oscillator(node) => match name {
                "frequency" => node.frequency().clone(),
                "detune" => node.detune().clone(),
                _ => unreachable!("unknown oscillator parameter"),
            },
            Self::Compressor { node, .. } => node.parameters()[name].clone(),
            Self::Biquad(node) => match name {
                "frequency" => node.frequency().clone(),
                "detune" => node.detune().clone(),
                "Q" => node.q().clone(),
                "gain" => node.gain().clone(),
                _ => unreachable!("unknown biquad parameter"),
            },
            _ => unreachable!("node has no parameters"),
        };
        let mut parameter = Parameter::new(native);
        match self {
            Self::Oscillator(node) if name == "frequency" => {
                let nyquist = node.node.context().sample_rate() / 2.0;
                parameter = parameter.with_range(-nyquist, nyquist);
            }
            Self::Compressor { .. } => parameter.fixed_rate = Some(AutomationRate::K),
            _ => {}
        }
        parameter
    }
}

struct CompressorProcessor {
    dsp: super::compressor::Compressor,
    reduction: Arc<AtomicU32>,
    demanded: Arc<AtomicBool>,
    sample_rate: f32,
    tail_frames: usize,
}

impl AudioWorkletProcessor for CompressorProcessor {
    type ProcessorOptions = (f32, Arc<AtomicU32>, Arc<AtomicBool>);

    fn constructor((sample_rate, reduction, demanded): Self::ProcessorOptions) -> Self {
        Self {
            dsp: super::compressor::Compressor::new(sample_rate),
            reduction,
            demanded,
            sample_rate,
            tail_frames: 0,
        }
    }

    fn parameter_descriptors() -> Vec<AudioParamDescriptor> {
        [
            ("threshold", -24.0, -100.0, 0.0),
            ("knee", 30.0, 0.0, 40.0),
            ("ratio", 12.0, 1.0, 20.0),
            ("attack", 0.003, 0.0, 1.0),
            ("release", 0.25, 0.0, 1.0),
        ]
        .into_iter()
        .map(
            |(name, default_value, min_value, max_value)| AudioParamDescriptor {
                name: name.into(),
                default_value,
                min_value,
                max_value,
                automation_rate: AutomationRate::K,
            },
        )
        .collect()
    }

    fn process<'a, 'b>(
        &mut self,
        inputs: &'b [&'a [&'a [f32]]],
        outputs: &'b mut [&'a mut [&'a mut [f32]]],
        params: AudioParamValues<'b>,
        _scope: &'b AudioWorkletGlobalScope,
    ) -> bool {
        // The backend processes orphan nodes too. Built-in compressors follow
        // Blink's pull demand; they are not automatically pulled worklets.
        if !self.demanded.load(Ordering::Acquire) {
            for channel in outputs[0].iter_mut() {
                channel.fill(0.0);
            }
            return false;
        }
        let input = inputs[0];
        if input.is_empty() {
            self.tail_frames += outputs[0][0].len();
        } else {
            self.tail_frames = 0;
        }
        let values =
            ["threshold", "knee", "ratio", "attack", "release"].map(|name| params.get(name)[0]);
        self.dsp.process(input, outputs[0], values);
        self.reduction
            .store(self.dsp.reduction().to_bits(), Ordering::Relaxed);
        self.tail_frames < (1.625 * self.sample_rate) as usize
    }
}

pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    state: State,
) {
    let store = scope.get_slot::<Store>().cloned().unwrap_or_else(|| {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    });
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("audio identity exhausted");
        store.next_id
    };
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        owner,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store
        .borrow_mut()
        .entries
        .insert(id, (weak, Rc::new(RefCell::new(state))));
    set_private_value(
        scope,
        owner,
        SLOT,
        v8::BigInt::new_from_u64(scope, id).into(),
    );
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<StateRef> {
    let id = v8::Local::<v8::BigInt>::try_from(get_private_value(scope, owner, SLOT)?)
        .ok()?
        .u64_value()
        .0;
    scope
        .get_slot::<Store>()?
        .borrow()
        .entries
        .get(&id)
        .map(|entry| entry.1.clone())
}

pub(super) fn create_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    kind: NodeKind,
) -> Node {
    let state = get(scope, context).expect("validated audio context must retain its backend");
    let state = state.borrow();
    let State::Context(context) = &*state else {
        unreachable!("audio context brand mismatch")
    };
    context.create_node(kind)
}

pub(super) fn param<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Parameter> {
    if let Some(state) = get(scope, object)
        && let State::Param(param) = &*state.borrow()
    {
        return Some(param.clone());
    }
    super::throw_type_error(scope, "Illegal invocation: expected an AudioParam.");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_realtime_context_closes_its_render_graph() {
        let context = Context::realtime();
        let destination = context.create_node(NodeKind::Destination);
        drop(context);
        assert_eq!(
            destination.audio_node().context().state(),
            web_audio_api::context::AudioContextState::Closed
        );
    }

    #[test]
    fn audio_backend_wrappers_are_reclaimed_on_gc_and_isolate_drop() {
        moli_v8_test_util::ensure_v8();
        let remaining = {
            let mut isolate = v8::Isolate::new(Default::default());
            let abandoned = {
                let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                let context = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, context);
                let context = v8::Object::new(scope);
                initialize(
                    scope,
                    context,
                    State::Context(Context::offline(1, 128, 8000.0)),
                );
                let node = create_node(scope, context, NodeKind::Oscillator);
                let param = node.param("frequency");
                let owner = v8::Object::new(scope);
                initialize(scope, owner, State::Node(node));
                let parameter = v8::Object::new(scope);
                initialize(scope, parameter, State::Param(param));
                [context, owner, parameter]
                    .map(|object| Rc::downgrade(&get(scope, object).unwrap()))
            };
            isolate.low_memory_notification();
            assert!(abandoned.iter().all(|state| state.upgrade().is_none()));
            assert!(
                isolate
                    .get_slot::<Store>()
                    .unwrap()
                    .borrow()
                    .entries
                    .is_empty()
            );
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let object = v8::Object::new(scope);
            initialize(
                scope,
                object,
                State::Context(Context::offline(1, 128, 8000.0)),
            );
            Rc::downgrade(&get(scope, object).unwrap())
        };
        assert!(remaining.upgrade().is_none());
    }
}
