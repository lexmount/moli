use super::*;

#[test]
fn web_audio_factories_are_shared_by_realtime_and_offline_contexts() {
    let mut vm = new_storage_test_vm("https://audio-factories.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const realtime = new AudioContext();
  const offline = new OfflineAudioContext(1, 64, 44100);
  return JSON.stringify(["createOscillator", "createAnalyser", "createDynamicsCompressor", "createBiquadFilter"].map(name => {
    const method = BaseAudioContext.prototype[name];
    const liveNode = realtime[name]();
    const offlineNode = offline[name]();
    return [
      method === realtime[name] && method === offline[name],
      Object.hasOwn(BaseAudioContext.prototype, name),
      Object.hasOwn(AudioContext.prototype, name),
      Object.hasOwn(OfflineAudioContext.prototype, name),
      method.length,
      liveNode.constructor.name,
      offlineNode.constructor.name,
      liveNode !== realtime[name]()
    ];
  }));
})()
"#,
        )
        .expect("both audio context kinds should expose shared factories");
    assert_eq!(
        result,
        r#"[[true,true,false,false,0,"OscillatorNode","OscillatorNode",true],[true,true,false,false,0,"AnalyserNode","AnalyserNode",true],[true,true,false,false,0,"DynamicsCompressorNode","DynamicsCompressorNode",true],[true,true,false,false,0,"BiquadFilterNode","BiquadFilterNode",true]]"#
    );
}

#[test]
fn web_audio_analyser_data_methods_are_shared_on_the_prototype() {
    let mut vm = new_storage_test_vm("https://audio-analyser-methods.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const context = new OfflineAudioContext(1, 64, 44100);
  const a = context.createAnalyser();
  const b = context.createAnalyser();
  return JSON.stringify([
    "getFloatFrequencyData", "getFloatTimeDomainData",
    "getByteFrequencyData", "getByteTimeDomainData"
  ].map(name => {
    const descriptor = Object.getOwnPropertyDescriptor(AnalyserNode.prototype, name);
    return [
      typeof AnalyserNode.prototype[name],
      a[name] === b[name] && a[name] === AnalyserNode.prototype[name],
      Object.hasOwn(a, name),
      descriptor && descriptor.enumerable,
      descriptor && descriptor.writable && descriptor.configurable,
      descriptor && descriptor.value.length,
      descriptor && descriptor.value.name === name
    ];
  }));
})()
"#,
        )
        .expect("analyser method descriptors should evaluate");
    assert_eq!(
        result,
        r#"[["function",true,false,true,true,1,true],["function",true,false,true,true,1,true],["function",true,false,true,true,1,true],["function",true,false,true,true,1,true]]"#
    );
}

#[test]
fn web_audio_factories_reject_forged_contexts_and_keep_nodes_independent() {
    let mut vm = new_storage_test_vm("https://audio-factory-receivers.test/");
    let result = vm.eval(r#"
(() => {
  const errors = [];
  for (const context of [new AudioContext(), new OfflineAudioContext(1, 64, 44100)]) {
    for (const name of ['createOscillator', 'createAnalyser', 'createDynamicsCompressor', 'createBiquadFilter']) {
      for (const receiver of [{}, BaseAudioContext.prototype, Object.create(context), new Proxy(context, {})]) {
        try { context[name].call(receiver); errors.push(name + ': accepted forged context'); }
        catch (error) { if (error.name !== 'TypeError') errors.push(name + ': ' + error.name); }
      }
    }
    const a = context.createOscillator(), b = context.createOscillator();
    a.type = 'triangle';
    if (a.frequency.setValueAtTime(880, 0) !== a.frequency) errors.push('AudioParam return value');
    if (a.frequency.value !== 880 || b.frequency.value !== 440 || b.type !== 'sine') errors.push('shared oscillator state');
    const analyser = context.createAnalyser();
    if (a.connect(analyser) !== analyser || analyser.connect(context.destination) !== context.destination) errors.push('connect return value');
    if (a.start(0) !== undefined || a.disconnect() !== undefined) errors.push('source return value');
  }
  if (AudioContext !== webkitAudioContext || typeof new webkitAudioContext().createOscillator !== 'function') errors.push('webkit alias');
  if (typeof AudioContext.prototype.startRendering !== 'undefined') errors.push('offline-only method leaked');
  return JSON.stringify(errors);
})()
"#).expect("audio factories should enforce receivers and keep per-node state");
    assert_eq!(result, "[]");
}

#[test]
fn web_audio_analyser_fft_size_validates_and_drives_readonly_bin_count() {
    let mut vm = new_storage_test_vm("https://audio-fft-size.test/");
    let result = vm.eval(r#"
(() => {
  const errors = [];
  for (const context of [new AudioContext(), new OfflineAudioContext(1, 64, 44100)]) {
    const analyser = context.createAnalyser(), other = context.createAnalyser();
    for (let size = 32; size <= 32768; size *= 2) {
      analyser.fftSize = size;
      if (analyser.fftSize !== size || analyser.frequencyBinCount !== size / 2) errors.push('size ' + size);
    }
    for (const size of [-1, 0, 16, 31, 33, 1000, 65536, NaN, Infinity, -Infinity, undefined, null]) {
      try { analyser.fftSize = size; errors.push('accepted ' + String(size)); }
      catch (error) { if (error.name !== 'IndexSizeError') errors.push('wrong exception ' + error.name); }
      if (analyser.fftSize !== 32768) errors.push('invalid assignment changed size');
    }
    for (const size of [Symbol(), 64n]) {
      try { analyser.fftSize = size; errors.push('accepted non-number'); }
      catch (error) { if (error.name !== 'TypeError') errors.push(error.name); }
    }
    analyser.fftSize = '64';
    if (analyser.frequencyBinCount !== 32) errors.push('string conversion');
    analyser.fftSize = 32.9;
    if (analyser.fftSize !== 32) errors.push('unsigned long conversion');
    if (Reflect.set(analyser, 'frequencyBinCount', 3) !== false || analyser.frequencyBinCount !== 16) errors.push('bin count is writable');
    const sentinel = new Error('conversion');
    try { analyser.fftSize = { valueOf() { throw sentinel; } }; errors.push('ignored conversion exception'); }
    catch (error) { if (error !== sentinel) errors.push('replaced conversion exception'); }
    if (other.fftSize !== 2048 || other.frequencyBinCount !== 1024) errors.push('shared analyser state');
  }
  return JSON.stringify(errors);
})()
"#).expect("analyser sizes should follow WebIDL conversion and validation");
    assert_eq!(result, "[]");
}

#[test]
fn web_audio_analyser_writes_only_available_samples_in_the_destination_view() {
    let mut vm = new_storage_test_vm("https://audio-analyser-buffer.test/");
    let result = vm.eval(r#"
(() => {
  const errors = [];
  for (const context of [new AudioContext(), new OfflineAudioContext(1, 64, 44100)]) {
    const analyser = context.createAnalyser();
    for (const size of [32, 2048, 32768]) {
      analyser.fftSize = size;
      for (const [method, ArrayType, count, expected] of [
        ['getFloatFrequencyData', Float32Array, size / 2, -Infinity],
        ['getFloatTimeDomainData', Float32Array, size, 0],
        ['getByteFrequencyData', Uint8Array, size / 2, 0],
        ['getByteTimeDomainData', Uint8Array, size, 128]
      ]) {
        for (const length of [0, 1, count - 1, count, count + 3]) {
          const storage = new ArrayType(length + 4).fill(57);
          const view = storage.subarray(2, 2 + length);
          Object.defineProperty(view, 'length', { get() { throw new Error('JS length must not be read'); } });
          if (analyser[method](view) !== undefined) errors.push(method + ': return');
          for (let i = 0; i < storage.length; i++) {
            const written = i >= 2 && i < 2 + Math.min(count, length);
            if (!written && storage[i] !== 57) errors.push(method + ': out-of-range write ' + i);
            if (written && storage[i] !== expected) errors.push(method + ': sample ' + i);
          }
        }
      }
    }
  }
  return JSON.stringify(errors);
})()
"#).expect("analyser reads should respect native view length, offset and sample count");
    assert_eq!(result, "[]");
}

#[test]
fn web_audio_analyser_checks_receiver_and_typed_array_without_duck_typing() {
    let mut vm = new_storage_test_vm("https://audio-analyser-arguments.test/");
    let result = vm.eval(r#"
(() => {
  const errors = [];
  const analyser = new AudioContext().createAnalyser();
  for (const [method, ArrayType, WrongArrayType] of [
    ['getFloatFrequencyData', Float32Array, Uint8Array],
    ['getFloatTimeDomainData', Float32Array, Uint8Array],
    ['getByteFrequencyData', Uint8Array, Float32Array],
    ['getByteTimeDomainData', Uint8Array, Float32Array]
  ]) {
    const expectTypeError = (run, label) => {
      try { run(); errors.push(method + ': accepted ' + label); }
      catch (error) { if (error.name !== 'TypeError') errors.push(method + ': ' + error.name); }
    };
    const fake = { get length() { throw new Error('must not read arbitrary length'); } };
    for (const value of [undefined, null, 1, [], fake, new WrongArrayType(4), new Float64Array(4), new Uint8ClampedArray(4), new DataView(new ArrayBuffer(16)), new Proxy(new ArrayType(4), {})]) {
      expectTypeError(() => analyser[method](value), 'wrong array');
    }
    expectTypeError(() => analyser[method](), 'missing argument');
    expectTypeError(() => analyser[method](new ArrayType(new SharedArrayBuffer(16))), 'shared buffer');
    expectTypeError(() => analyser[method](new ArrayType(new ArrayBuffer(16, { maxByteLength: 32 }))), 'resizable buffer');
    const output = new ArrayType(4).fill(57);
    for (const receiver of [{}, AnalyserNode.prototype, Object.create(analyser), new Proxy(analyser, {})]) {
      expectTypeError(() => analyser[method].call(receiver, output), 'forged receiver');
    }
    if (!output.every(value => value === 57)) errors.push(method + ': invalid invocation wrote data');
  }
  const size = Object.getOwnPropertyDescriptor(AnalyserNode.prototype, 'fftSize');
  const bins = Object.getOwnPropertyDescriptor(AnalyserNode.prototype, 'frequencyBinCount');
  for (const run of [() => size.get.call({}), () => size.set.call({}, 32), () => bins.get.call({})]) {
    try { run(); errors.push('accepted forged accessor receiver'); }
    catch (error) { if (error.name !== 'TypeError') errors.push(error.name); }
  }
  return JSON.stringify(errors);
})()
"#).expect("analyser data methods should reject incompatible receivers and buffers");
    assert_eq!(result, "[]");
}

#[test]
fn web_audio_biquad_and_existing_params_expose_native_readonly_metadata() {
    let mut vm = new_storage_test_vm("https://audio-parameter-metadata.test/");
    let result = vm.eval(r#"
(() => {
  const errors = [];
  for (const rate of [8000, 44100, 48000]) {
    const ctx = new OfflineAudioContext(1, 64, rate);
    ctx.sampleRate = 123; // Native bounds must not depend on an overridable property.
    const filter = ctx.createBiquadFilter(), other = ctx.createBiquadFilter();
    if (!(filter instanceof BiquadFilterNode) || Object.prototype.toString.call(filter) !== '[object BiquadFilterNode]') errors.push('brand');
    if (filter.type !== 'lowpass' || filter.frequency !== filter.frequency || filter.frequency === other.frequency) errors.push('node state');
    if (Reflect.set(filter, 'frequency', {}) !== false) errors.push('writable frequency');
    const osc = ctx.createOscillator(), comp = ctx.createDynamicsCompressor();
    for (const node of [filter, osc, comp, ctx.createAnalyser(), ctx.destination]) {
      if (node.context !== ctx || Reflect.set(node, 'context', {}) !== false) errors.push('context');
    }
    const rows = [
      [filter.frequency, 350, 0, rate / 2],
      [filter.Q, 1, -3.4028234663852886e38, 3.4028234663852886e38],
      [filter.detune, 0, -153600, 153600],
      [osc.frequency, 440, -rate / 2, rate / 2],
      [osc.detune, 0, -153600, 153600],
      [comp.threshold, -24, -100, 0], [comp.knee, 30, 0, 40],
      [comp.ratio, 12, 1, 20], [comp.attack, Math.fround(.003), 0, 1],
      [comp.release, .25, 0, 1]
    ];
    for (const [p, value, min, max] of rows) {
      if (!(p instanceof AudioParam) || p.value !== value || p.defaultValue !== value || p.minValue !== min || p.maxValue !== max) errors.push('metadata ' + value);
      p.value = 123;
      if (p.defaultValue !== value) errors.push('default changed');
      for (const key of ['defaultValue', 'minValue', 'maxValue']) {
        const descriptor = Object.getOwnPropertyDescriptor(AudioParam.prototype, key);
        if (!descriptor || !descriptor.enumerable || descriptor.set || Reflect.set(p, key, 999) !== false) errors.push('readonly ' + key);
        for (const receiver of [{}, AudioParam.prototype, Object.create(p), new Proxy(p, {})]) {
          try { descriptor.get.call(receiver); errors.push('accepted forged param'); }
          catch (e) { if (e.name !== 'TypeError') errors.push(e.name); }
        }
      }
    }
    if (filter.gain.defaultValue !== 0 || filter.gain.minValue !== Math.fround(-3.4028234663852886e38) || !(filter.gain.maxValue > 1541 && filter.gain.maxValue < 1542)) errors.push('gain range');
    try { filter.getFrequencyResponse(new Float32Array(1), new Float32Array(1), new Float32Array(1)); errors.push('fabricated frequency response'); }
    catch (e) { if (e.name !== 'NotSupportedError') errors.push('response ' + e.name); }
  }
  return JSON.stringify(errors);
})()
"#).expect("BiquadFilter and AudioParam metadata should be native and context-specific");
    assert_eq!(result, "[]");
}

#[test]
fn offline_audio_silence_depends_on_reachable_started_sources() {
    let mut vm = new_storage_test_vm("https://audio-graph-silence.test/");
    vm.exec(r#"
globalThis.__audioSilenceResults = [];
for (const mode of ['empty', 'unconnected', 'not-started', 'future', 'disconnected-source', 'disconnected-sink', 'cycle', 'connected', 'duplicate', 'selected-disconnect']) {
  const ctx = new OfflineAudioContext(1, 100, 44100);
  const osc = ctx.createOscillator(), comp = ctx.createDynamicsCompressor();
  if (mode !== 'empty' && mode !== 'unconnected') {
    osc.connect(comp);
    comp.connect(ctx.destination);
  }
  if (mode !== 'empty' && mode !== 'not-started') osc.start(mode === 'future' ? 1 : 0);
  if (mode === 'disconnected-source') osc.disconnect();
  if (mode === 'disconnected-sink') comp.disconnect(ctx.destination);
  if (mode === 'cycle') { osc.disconnect(); const a = ctx.createAnalyser(); comp.connect(a); a.connect(comp); }
  if (mode === 'duplicate') { osc.connect(comp); osc.connect(comp); }
  if (mode === 'selected-disconnect') { const a = ctx.createAnalyser(); osc.connect(a); osc.disconnect(a); }
  ctx.startRendering().then(buffer => {
    __audioSilenceResults.push([mode, buffer.getChannelData(0).some(x => x !== 0), comp.reduction < 0]);
  });
}
"#, None).expect("offline graph scenarios should render without hanging on a cycle");
    let result = vm.eval("JSON.stringify(__audioSilenceResults)").unwrap();
    assert_eq!(
        result,
        r#"[["empty",false,false],["unconnected",false,false],["not-started",false,false],["future",false,false],["disconnected-source",false,false],["disconnected-sink",false,false],["cycle",false,false],["connected",true,true],["duplicate",true,true],["selected-disconnect",true,true]]"#
    );
}

#[test]
fn offline_analyser_starts_silent_and_retains_the_rendered_input_snapshot() {
    let mut vm = new_storage_test_vm("https://audio-analyser-silence.test/");
    vm.exec(r#"
const ctx = new OfflineAudioContext(1, 5000, 44100);
const osc = ctx.createOscillator(), comp = ctx.createDynamicsCompressor();
const analyser = ctx.createAnalyser(), silent = ctx.createAnalyser();
const isSilent = node => { const data = new Float32Array(16); node.getFloatFrequencyData(data); return data.every(x => x === -Infinity); };
globalThis.__analyserSilence = [isSilent(analyser)];
osc.connect(comp); comp.connect(analyser); comp.connect(ctx.destination); osc.start();
__analyserSilence.push(isSilent(analyser));
ctx.oncomplete = () => {
  comp.disconnect(); osc.disconnect();
  __analyserSilence.push(isSilent(analyser), isSilent(silent));
};
ctx.startRendering();
"#, None).expect("analyser silence and retained rendering state should evaluate");
    assert_eq!(
        vm.eval("JSON.stringify(__analyserSilence)").unwrap(),
        "[true,true,false,true]"
    );
}

#[test]
fn web_audio_connections_validate_context_and_source_receivers() {
    let mut vm = new_storage_test_vm("https://audio-connection-receivers.test/");
    let result = vm.eval(r#"
(() => {
  const ctx = new AudioContext(), other = new AudioContext(), osc = ctx.createOscillator();
  const errors = [];
  const check = (fn, expected) => { try { fn(); errors.push('accepted ' + expected); } catch (e) { if (e.name !== expected) errors.push(e.name); } };
  check(() => osc.connect(other.destination), 'InvalidAccessError');
  check(() => osc.connect({}), 'TypeError');
  check(() => osc.connect(ctx.destination, 1), 'IndexSizeError');
  check(() => osc.connect(new Proxy(ctx.destination, {})), 'TypeError');
  for (const receiver of [{}, Object.create(osc), new Proxy(osc, {})]) {
    check(() => osc.connect.call(receiver, ctx.destination), 'TypeError');
    check(() => osc.disconnect.call(receiver), 'TypeError');
    check(() => osc.start.call(receiver), 'TypeError');
  }
  check(() => osc.start(-1), 'RangeError');
  check(() => osc.start(Infinity), 'TypeError');
  osc.start();
  check(() => osc.start(), 'InvalidStateError');
  return JSON.stringify(errors);
})()
"#).expect("audio graph operations should validate receivers and context ownership");
    assert_eq!(result, "[]");
}
