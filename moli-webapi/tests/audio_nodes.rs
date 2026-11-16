mod support;
#[path = "support/targets.rs"]
mod targets;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{global_constructor_prototype, set_private_value, v8str};
use moli_webapi::{interfaces, web_audio as audio};
use std::pin::pin;
use support::{eval, finish_template, illegal_constructor, install_errors, publish};

fn install<'s>(scope: &mut v8::PinScope<'s, '_>) {
    install_errors(scope);
    targets::install(scope);
    let target = v8::FunctionTemplate::new(scope, illegal_constructor);
    finish_template(scope, interfaces::EventTarget::DESCRIPTOR, target);
    let context = v8::FunctionTemplate::new(scope, illegal_constructor);
    context.inherit(target);
    let prototype = context.prototype_template(scope);
    for (name, template) in [
        (
            "createGain",
            v8::FunctionTemplate::new(scope, audio::gain::create),
        ),
        (
            "createAnalyser",
            v8::FunctionTemplate::new(scope, audio::audio_context_create_analyser_callback),
        ),
        (
            "createDynamicsCompressor",
            v8::FunctionTemplate::new(
                scope,
                audio::audio_context_create_dynamics_compressor_callback,
            ),
        ),
        (
            "createBiquadFilter",
            v8::FunctionTemplate::new(scope, audio::biquad::create_biquad_filter),
        ),
    ] {
        prototype.set(v8str(scope, name).into(), template.into());
    }
    let listener = v8::FunctionTemplate::new(scope, audio::listener::get);
    prototype.set_accessor_property(
        v8str(scope, "listener").into(),
        Some(listener),
        None,
        Default::default(),
    );
    finish_template(scope, interfaces::BaseAudioContext::DESCRIPTOR, context);
    let realtime = v8::FunctionTemplate::new(scope, illegal_constructor);
    realtime.inherit(context);
    finish_template(scope, interfaces::AudioContext::DESCRIPTOR, realtime);
    let offline = v8::FunctionTemplate::new(scope, illegal_constructor);
    offline.inherit(context);
    finish_template(scope, interfaces::OfflineAudioContext::DESCRIPTOR, offline);

    let node = v8::FunctionTemplate::new(scope, illegal_constructor);
    node.inherit(target);
    audio::install(scope, node, "AudioNode");
    finish_template(scope, interfaces::AudioNode::DESCRIPTOR, node);
    let scheduled = v8::FunctionTemplate::new(scope, illegal_constructor);
    scheduled.inherit(node);
    audio::install(scope, scheduled, "AudioScheduledSourceNode");
    finish_template(
        scope,
        interfaces::AudioScheduledSourceNode::DESCRIPTOR,
        scheduled,
    );
    for (interface, template, parent) in [
        (
            interfaces::AudioBuffer::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::buffer::constructor),
            None,
        ),
        (
            interfaces::AudioParam::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, illegal_constructor),
            None,
        ),
        (
            interfaces::AudioListener::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, illegal_constructor),
            None,
        ),
        (
            interfaces::PeriodicWave::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::periodic_wave::constructor),
            None,
        ),
        (
            interfaces::AudioDestinationNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, illegal_constructor),
            Some(node),
        ),
        (
            interfaces::MediaElementAudioSourceNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::media_element_source::constructor),
            Some(node),
        ),
        (
            interfaces::GainNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::gain::constructor),
            Some(node),
        ),
        (
            interfaces::BiquadFilterNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, illegal_constructor),
            Some(node),
        ),
        (
            interfaces::AnalyserNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, illegal_constructor),
            Some(node),
        ),
        (
            interfaces::DynamicsCompressorNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, illegal_constructor),
            Some(node),
        ),
        (
            interfaces::DelayNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::param_nodes::delay_constructor),
            Some(node),
        ),
        (
            interfaces::StereoPannerNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::param_nodes::stereo_panner_constructor),
            Some(node),
        ),
        (
            interfaces::ConvolverNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::convolver::constructor),
            Some(node),
        ),
        (
            interfaces::IIRFilterNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::iir::constructor),
            Some(node),
        ),
        (
            interfaces::WaveShaperNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::wave_shaper::constructor),
            Some(node),
        ),
        (
            interfaces::ChannelMergerNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::channel_nodes::channel_merger_constructor),
            Some(node),
        ),
        (
            interfaces::ChannelSplitterNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::channel_nodes::channel_splitter_constructor),
            Some(node),
        ),
        (
            interfaces::AudioBufferSourceNode::DESCRIPTOR,
            v8::FunctionTemplate::new(
                scope,
                audio::source::audio_buffer_source_constructor_callback,
            ),
            Some(scheduled),
        ),
        (
            interfaces::ConstantSourceNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::source::constant_source_constructor_callback),
            Some(scheduled),
        ),
        (
            interfaces::OscillatorNode::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, audio::oscillator::constructor),
            Some(scheduled),
        ),
    ] {
        if let Some(parent) = parent {
            template.inherit(parent);
        }
        audio::install(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    for (name, interface, rate) in [
        ("context", "AudioContext", None),
        ("otherContext", "AudioContext", None),
        ("offline", "OfflineAudioContext", Some(48_000.0)),
    ] {
        let object = v8::Object::new(scope);
        let prototype = global_constructor_prototype(scope, interface).unwrap();
        object.set_prototype(scope, prototype.into()).unwrap();
        interfaces::initialize(scope, object, interface).unwrap();
        if let Some(rate) = rate {
            audio::set_web_audio_number_slot(
                scope,
                object,
                audio::OFFLINE_AUDIO_SAMPLE_RATE_SLOT,
                rate,
            );
        }
        let destination = audio::audio_destination_node(scope, object);
        object
            .set(
                scope,
                v8str(scope, "destination").into(),
                destination.into(),
            )
            .unwrap();
        publish(scope, name, object);
    }
    eval(
        scope,
        r#"
        function check(ok, label) { if (!ok) throw Error(label); }
        function throws(name, run) {
          try { run(); } catch (e) { check(e.name === name, name + ': ' + e); return e; }
          throw Error('missing ' + name);
        }
    "#,
    );
}

fn run(source: &str) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    eval(scope, source);
}

#[test]
fn audio_graph_ports_contexts_and_native_parameters_survive_extraction() {
    run(r#"
      const gain = new GainNode(context, {gain: .25, channelCount: 1});
      check(gain instanceof AudioNode && gain instanceof EventTarget, 'inheritance');
      check(gain.context === context && gain.gain.value === .25, 'owner and gain');
      check(gain.gain === gain.gain && gain.gain instanceof AudioParam, 'param identity');
      check(gain.channelCount === 1 && gain.numberOfInputs === 1, 'metadata');
      check(gain.connect(context.destination) === context.destination, 'connection result');
      throws('InvalidAccessError', () => gain.connect(new GainNode(otherContext)));
      throws('IndexSizeError', () => gain.connect(context.destination, 2));
      gain.disconnect(context.destination);
      throws('InvalidAccessError', () => gain.disconnect(context.destination));
      const merger = new ChannelMergerNode(context, {numberOfInputs: 3});
      const splitter = new ChannelSplitterNode(context, {numberOfOutputs: 4});
      check(merger.numberOfInputs === 3 && merger.numberOfOutputs === 1, 'merger ports');
      check(splitter.numberOfOutputs === 4 && splitter.channelCount === 4, 'splitter ports');
      check(context.destination.numberOfOutputs === 0, 'terminal destination');
      check(context.listener === context.listener && context.listener instanceof AudioListener, 'listener');
      check(context.listener.forwardZ.value === -1, 'listener orientation');
      const getContext = Object.getOwnPropertyDescriptor(AudioNode.prototype, 'context').get;
      throws('TypeError', () => getContext.call(Object.create(GainNode.prototype)));
    "#);
}

#[test]
fn audio_buffers_preserve_channels_copy_offsets_and_acquisition() {
    run(r#"
      const buffer = new AudioBuffer({length: 4, numberOfChannels: 2, sampleRate: 48000});
      const samples = buffer.getChannelData(0);
      check(samples === buffer.getChannelData(0), 'same view before acquisition');
      buffer.copyToChannel(new Float32Array([.25, .5]), 0, 1);
      check(samples.join(',') === '0,0.25,0.5,0', 'writes share backing');
      const out = new Float32Array([9,9,9,9]);
      buffer.copyFromChannel(out, 0, 2);
      check(out.join(',') === '0.5,0,9,9', 'partial copy');
      throws('IndexSizeError', () => buffer.getChannelData(2));
      throws('TypeError', () => buffer.copyToChannel(new Uint8Array(2), 0));
      const source = new AudioBufferSourceNode(context, {buffer});
      source.start();
      check(samples.byteLength === 0, 'start acquires old channel views');
      check(buffer.getChannelData(0).join(',') === '0,0.25,0.5,0', 'new view retains samples');
      check(buffer.getChannelData(1).every(x => x === 0), 'separate zero channel');
      check(buffer.length === 4 && buffer.sampleRate === 48000, 'native dimensions');
    "#);
}

#[test]
fn scheduled_sources_keep_conversion_order_and_handler_handoff() {
    run(r#"
      const source = new ConstantSourceNode(context, {offset: 2});
      let conversions = 0;
      throws('RangeError', () => source.start({valueOf() { conversions++; return -1; }}));
      check(conversions === 1, 'time converted once');
      source.start(0);
      throws('InvalidStateError', () => source.start(0));
      source.stop(1);
      const handler = () => {};
      source.onended = handler;
      check(source.onended === handler, 'host stores handler');
      source.onended = 1;
      check(source.onended === null, 'non-object handler');
      const oscillator = new OscillatorNode(context, {frequency: 220, type: 'triangle'});
      check(oscillator instanceof AudioScheduledSourceNode, 'scheduled inheritance');
      check(oscillator.frequency.value === 220 && oscillator.type === 'triangle', 'options');
      const wave = new PeriodicWave(context, {real: [0,1], imag: [0,0]});
      oscillator.setPeriodicWave(wave);
      check(oscillator.type === 'custom', 'native periodic wave');
      throws('TypeError', () => oscillator.setPeriodicWave(Object.create(PeriodicWave.prototype)));
    "#);
}

#[test]
fn audio_leaf_options_and_private_sample_rate_ignore_public_shadows() {
    run(r#"
      Object.defineProperty(offline, 'sampleRate', {get() {throw Error('public rate');}});
      const delay = new DelayNode(offline, {maxDelayTime: 2, delayTime: .75});
      check(delay.delayTime.maxValue === 2 && delay.delayTime.value === .75, 'delay bounds');
      const panner = new StereoPannerNode(offline, {pan: -.5});
      check(panner.pan.value === -.5 && panner.channelCountMode === 'clamped-max', 'panner');
      const convolver = new ConvolverNode(offline, {disableNormalization: true});
      check(convolver.normalize === false && convolver.buffer === null, 'convolver');
      const curve = new Float32Array([-1, 0, 1]);
      const shaper = new WaveShaperNode(offline, {curve});
      check(shaper.curve !== curve && shaper.curve.join(',') === '-1,0,1', 'curve copy');
      const filter = new IIRFilterNode(offline, {feedforward: [1], feedback: [1]});
      check(filter instanceof AudioNode && filter.context === offline, 'IIR owner');
      const frequency = new Float32Array([0, 100, 24001]);
      const magnitude = new Float32Array(3), phase = new Float32Array(3);
      filter.getFrequencyResponse(frequency, magnitude, phase);
      check(magnitude[0] === 1 && magnitude[1] === 1 && Number.isNaN(magnitude[2]), 'native Nyquist');
      check(context.createBiquadFilter().frequency.value === 350, 'biquad factory');
      check(context.createDynamicsCompressor().threshold.value === -24, 'compressor factory');
    "#);
}

#[test]
fn audio_native_factories_ignore_replaced_public_constructors() {
    run(r#"
      const prototype = GainNode.prototype;
      let reads = 0;
      Object.defineProperty(globalThis, 'GainNode', {get() {reads++; throw Error('author constructor');}, configurable: true});
      const gain = context.createGain();
      check(Object.getPrototypeOf(gain) === prototype && gain.gain.value === 1, 'canonical factory');
      check(reads === 0, 'no public lookup');
      const analyser = context.createAnalyser();
      analyser.fftSize = 32;
      const bytes = new Uint8Array(40).fill(9);
      analyser.getByteTimeDomainData(bytes);
      check(bytes.slice(0,32).every(x => x === 128) && bytes.slice(32).every(x => x === 9), 'bounded silent analyser');
      throws('IndexSizeError', () => { analyser.fftSize = 33; });
    "#);
}

#[test]
fn audio_media_source_delegates_native_ownership_before_graph_binding() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    // This embedder has one fake native element. Renderer integration tests
    // cover real DOM wrappers, adoption and cross-world uniqueness.
    audio::install_runtime(
        scope,
        audio::AudioRuntime {
            initialize_media_source: bind_media_source,
        },
    );
    let media = v8::FunctionTemplate::new(scope, illegal_constructor);
    finish_template(scope, interfaces::HTMLMediaElement::DESCRIPTOR, media);
    let element = v8::Object::new(scope);
    interfaces::initialize(scope, element, "HTMLMediaElement").unwrap();
    publish(scope, "element", element);
    eval(
        scope,
        r#"
      const source = new MediaElementAudioSourceNode(context, {mediaElement: element});
      check(source.mediaElement === element && source.context === context, 'native handoff');
      check(source.numberOfInputs === 0, 'media source ports');
      throws('InvalidStateError', () => new MediaElementAudioSourceNode(context, {mediaElement: element}));
    "#,
    );
}

fn bind_media_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
) -> bool {
    const BOUND: &str = "testNativeAudioSource";
    if moli_v8_util::get_private_value(scope, element, BOUND).is_some() {
        moli_webapi::dom_exception::throw_dom_exception_value(
            scope,
            "already bound",
            "InvalidStateError",
        );
        return false;
    }
    if !audio::media_element_source::bind_native_source(scope, node, context, element) {
        return false;
    }
    set_private_value(scope, element, BOUND, node.into());
    true
}
