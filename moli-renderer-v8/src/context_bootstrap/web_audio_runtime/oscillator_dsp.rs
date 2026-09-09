use super::wavetable::{self, Tables};
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
};
use web_audio_api::context::BaseAudioContext;
use web_audio_api::node::OscillatorType;
use web_audio_api::worklet::{
    AudioParamValues, AudioWorkletGlobalScope, AudioWorkletNode, AudioWorkletNodeOptions,
    AudioWorkletProcessor,
};
use web_audio_api::{AudioParam, AudioParamDescriptor, AutomationRate};

struct Controls {
    shape: AtomicU8,
    start: AtomicU64,
}

pub(super) struct Oscillator {
    pub(super) node: AudioWorkletNode,
    controls: Arc<Controls>,
}

impl Oscillator {
    pub(super) fn new<C: BaseAudioContext>(context: &C) -> Self {
        let controls = Arc::new(Controls {
            shape: AtomicU8::new(0),
            start: AtomicU64::new(f64::INFINITY.to_bits()),
        });
        let tables = wavetable::for_sample_rate(context.sample_rate());
        let node = AudioWorkletNode::new::<Processor>(
            context,
            AudioWorkletNodeOptions {
                number_of_inputs: 0,
                number_of_outputs: 1,
                output_channel_count: vec![1],
                processor_options: (Arc::clone(&controls), tables),
                parameter_data: Default::default(),
                audio_node_options: Default::default(),
            },
        );
        Self { node, controls }
    }
    pub(super) fn frequency(&self) -> &AudioParam {
        &self.node.parameters()["frequency"]
    }
    pub(super) fn detune(&self) -> &AudioParam {
        &self.node.parameters()["detune"]
    }
    pub(super) fn start_at(&mut self, when: f64) {
        self.controls.start.store(
            when.max(self.node.context().current_time()).to_bits(),
            Ordering::Release,
        );
    }
    pub(super) fn type_(&self) -> OscillatorType {
        match self.controls.shape.load(Ordering::Relaxed) {
            0 => OscillatorType::Sine,
            1 => OscillatorType::Square,
            2 => OscillatorType::Sawtooth,
            3 => OscillatorType::Triangle,
            _ => unreachable!(),
        }
    }
    pub(super) fn set_type(&mut self, kind: OscillatorType) {
        let value = match kind {
            OscillatorType::Sine => 0,
            OscillatorType::Square => 1,
            OscillatorType::Sawtooth => 2,
            OscillatorType::Triangle => 3,
            OscillatorType::Custom => unreachable!("custom waves require their own table"),
        };
        self.controls.shape.store(value, Ordering::Release);
    }
}

struct Processor {
    controls: Arc<Controls>,
    tables: [Arc<Tables>; 4],
    position: f64,
    started: bool,
}

impl AudioWorkletProcessor for Processor {
    type ProcessorOptions = (Arc<Controls>, [Arc<Tables>; 4]);
    fn constructor((controls, tables): Self::ProcessorOptions) -> Self {
        Self {
            controls,
            tables,
            position: 0.0,
            started: false,
        }
    }
    fn parameter_descriptors() -> Vec<AudioParamDescriptor> {
        [
            ("frequency", 440.0, f32::MIN, f32::MAX),
            ("detune", 0.0, -153600.0, 153600.0),
        ]
        .into_iter()
        .map(
            |(name, default_value, min_value, max_value)| AudioParamDescriptor {
                name: name.into(),
                default_value,
                min_value,
                max_value,
                automation_rate: AutomationRate::A,
            },
        )
        .collect()
    }
    fn process<'a, 'b>(
        &mut self,
        _inputs: &'b [&'a [&'a [f32]]],
        outputs: &'b mut [&'a mut [&'a mut [f32]]],
        params: AudioParamValues<'b>,
        scope: &'b AudioWorkletGlobalScope,
    ) -> bool {
        let start = f64::from_bits(self.controls.start.load(Ordering::Acquire));
        let output = &mut outputs[0][0];
        if !start.is_finite() {
            output.fill(0.0);
            return false;
        }
        let table = &self.tables[self.controls.shape.load(Ordering::Acquire) as usize];
        let frequency = params.get("frequency");
        let detune = params.get("detune");
        let sample_rate = scope.sample_rate;
        let nyquist = sample_rate / 2.0;
        let rate_scale = table.size as f32 / sample_rate;
        for (index, sample) in output.iter_mut().enumerate() {
            let time = scope.current_time + index as f64 / f64::from(sample_rate);
            if time < start {
                *sample = 0.0;
                continue;
            }
            let hz = frequency[index.min(frequency.len() - 1)].clamp(-nyquist, nyquist);
            let detune = detune[index.min(detune.len() - 1)];
            let hz = ((f64::from(hz) * 2.0_f64.powf(f64::from(detune) / 1200.0)) as f32)
                .clamp(-nyquist, nyquist);
            let increment = f64::from(hz * rate_scale);
            if !self.started {
                self.position = (increment * ((time - start) * f64::from(sample_rate)))
                    .rem_euclid(table.size as f64);
                self.started = true;
            }
            *sample = table.sample(self.position, hz, sample_rate);
            self.position = (self.position + increment).rem_euclid(table.size as f64);
        }
        true
    }
}

use web_audio_api::node::AudioNode;
