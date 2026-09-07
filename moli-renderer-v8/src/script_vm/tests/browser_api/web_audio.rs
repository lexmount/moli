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
  return JSON.stringify(["createOscillator", "createAnalyser", "createDynamicsCompressor"].map(name => {
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
        r#"[[true,true,false,false,0,"OscillatorNode","OscillatorNode",true],[true,true,false,false,0,"AnalyserNode","AnalyserNode",true],[true,true,false,false,0,"DynamicsCompressorNode","DynamicsCompressorNode",true]]"#
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
    for (const name of ['createOscillator', 'createAnalyser', 'createDynamicsCompressor']) {
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
        ['getFloatFrequencyData', Float32Array, size / 2, null],
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
            if (written && (expected === null ? !Number.isFinite(storage[i]) || storage[i] >= 0 : storage[i] !== expected)) errors.push(method + ': sample ' + i);
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
