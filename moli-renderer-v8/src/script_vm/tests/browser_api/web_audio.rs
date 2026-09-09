use super::*;

#[test]
fn web_audio_connections_revalidate_after_coercion_without_page_hooks() {
    let mut vm = new_storage_test_vm("https://audio-connection-reentrancy.test/");
    let source = include_str!("../../../../tests/fixtures/audio-graph-reentrancy-check.js");
    assert_eq!(vm.eval(&format!("JSON.stringify({source})")).unwrap(), "[]");
}

#[test]
fn web_audio_native_state_boundaries_match_chromium() {
    let mut vm = new_storage_test_vm("https://audio-native-boundaries.test/");
    let source = include_str!("../../../../tests/fixtures/offline-audio-boundaries-check.js");
    vm.exec(
        &format!("({source}).then(value => globalThis.__audioBoundaries = value)"),
        None,
    )
    .expect("audio boundary fixture should execute");
    assert_eq!(vm.eval("JSON.stringify(__audioBoundaries)").unwrap(), "[]");
}

#[test]
fn offline_audio_analyser_uses_rendered_pcm_and_a_real_fft() {
    let mut vm = new_storage_test_vm("https://audio-rendered-pcm.test/");
    let source = include_str!("../../../../tests/fixtures/offline-audio-analyser-check.js");
    vm.exec(
        &format!("({source}).then(value => globalThis.__renderedPcm = value)"),
        None,
    )
    .expect("offline audio fixture should execute");
    let result = vm
        .eval("JSON.stringify(globalThis.__renderedPcm)")
        .expect("offline rendering should settle its promise");
    let rows: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 3);
    for row in rows.as_array().unwrap() {
        for key in [
            "waveformCorrect",
            "timeMatchesPcm",
            "retainsAfterDisconnect",
            "peakCorrect",
            "blackmanAmplitudeCorrect",
            "frequencyReadStable",
            "byteFrequencyCorrect",
            "byteTimeCorrect",
        ] {
            assert_eq!(row[key], true, "{key}: {row}");
        }
    }
}

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
    // Scheduling does not synchronously change .value (also true in Chromium).
    if (a.frequency.value !== 440 || b.frequency.value !== 440 || b.type !== 'sine') errors.push('premature automation');
    a.frequency.value = 880;
    if (a.frequency.value !== 880 || b.frequency.value !== 440) errors.push('shared oscillator state');
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
    filter.type = 'allpass';
    const magnitudes = new Float32Array(3), phases = new Float32Array(3);
    filter.getFrequencyResponse(new Float32Array([0, 350, rate / 4]), magnitudes, phases);
    if (!magnitudes.every(value => Math.abs(value - 1) < 1e-6) || !phases.every(Number.isFinite)) errors.push('allpass response');
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
for (const length of [100, 1024]) {
for (const mode of ['empty', 'unconnected', 'not-started', 'future', 'disconnected-source', 'disconnected-sink', 'cycle', 'connected', 'duplicate', 'selected-disconnect']) {
  const ctx = new OfflineAudioContext(1, length, 44100);
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
    const pcm = buffer.getChannelData(0);
    __audioSilenceResults.push([length, mode, pcm.some(x => x !== 0), pcm.subarray(0, Math.min(264, length)).every(x => x === 0)]);
  });
}
}
"#, None).expect("offline graph scenarios should render without hanging on a cycle");
    let result = vm.eval("JSON.stringify(__audioSilenceResults)").unwrap();
    assert_eq!(
        result,
        r#"[[100,"empty",false,true],[100,"unconnected",false,true],[100,"not-started",false,true],[100,"future",false,true],[100,"disconnected-source",false,true],[100,"disconnected-sink",false,true],[100,"cycle",false,true],[100,"connected",false,true],[100,"duplicate",false,true],[100,"selected-disconnect",false,true],[1024,"empty",false,true],[1024,"unconnected",false,true],[1024,"not-started",false,true],[1024,"future",false,true],[1024,"disconnected-source",false,true],[1024,"disconnected-sink",false,true],[1024,"cycle",false,true],[1024,"connected",true,true],[1024,"duplicate",true,true],[1024,"selected-disconnect",true,true]]"#
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
fn offline_compressor_meter_follows_pull_demand_not_input_reachability() {
    let mut vm = new_storage_test_vm("https://audio-compressor-demand.test/");
    vm.exec(r#"
globalThis.__meter = [];
for (const mode of ['orphan', 'silent-destination', 'source-without-destination', 'source-to-destination']) {
  const context = new OfflineAudioContext(1, 1024, 44100);
  const compressor = context.createDynamicsCompressor(), oscillator = context.createOscillator();
  if (mode.startsWith('source')) { oscillator.connect(compressor); oscillator.start(); }
  if (mode.endsWith('destination') && mode !== 'source-without-destination') compressor.connect(context.destination);
  const before = compressor.reduction;
  context.startRendering().then(buffer => __meter.push([mode, before, compressor.reduction < 0, buffer.getChannelData(0).some(x => x !== 0)]));
}
"#, None).unwrap();
    // Blink can have negative metering while processing silence. The old
    // reachability-based "no source => zero reduction" assertion was incorrect.
    assert_eq!(
        vm.eval("JSON.stringify(__meter)").unwrap(),
        r#"[["orphan",0,false,false],["silent-destination",0,true,false],["source-without-destination",0,false,false],["source-to-destination",0,true,true]]"#
    );
}

#[test]
fn offline_audio_automation_and_delayed_start_use_the_sample_clock() {
    let mut vm = new_storage_test_vm("https://audio-automation.test/");
    vm.exec(
        r#"
const context = new OfflineAudioContext(1, 4096, 8192), oscillator = context.createOscillator();
oscillator.frequency.value = 256;
oscillator.frequency.setValueAtTime(512, .25);
oscillator.detune.setValueAtTime(1200, .375);
oscillator.connect(context.destination);
oscillator.start(.125);
const before = oscillator.frequency.value;
context.startRendering().then(buffer => {
  const pcm = buffer.getChannelData(0);
  const expected = i => i < 1024 ? 0 :
    Math.sin(2 * Math.PI * (i < 2048 ? 256 : i < 3072 ? 512 : 1024) * i / 8192);
  globalThis.__automation = {
    before,
    error: Math.max(...pcm.map((value, i) => Math.abs(value - expected(i)))),
    silentPrefix: pcm.subarray(0, 1024).every(value => value === 0),
    state: context.state, time: context.currentTime
  };
});
"#,
        None,
    )
    .unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(__automation)").unwrap()).unwrap();
    assert_eq!(result["before"], 256);
    assert!(result["error"].as_f64().unwrap() < 1e-4, "{result}");
    assert_eq!(result["silentPrefix"], true);
    assert_eq!(result["state"], "closed");
    assert_eq!(result["time"], 0.5);
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
