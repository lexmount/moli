(async () => {
  const failures = [];
  const check = (name, value) => { if (!value) failures.push(name); };
  const throws = (name, action, expected) => {
    try { action(); failures.push(name + ': did not throw'); }
    catch (error) { if (error.name !== expected) failures.push(name + ': ' + error.name); }
  };
  for (const [channels, length, rate, error] of [
    [0, 128, 8000, 'NotSupportedError'], [33, 128, 8000, 'NotSupportedError'],
    [1, 0, 8000, 'NotSupportedError'], [NaN, 128, 8000, 'NotSupportedError'],
    [1, 128, 2999, 'NotSupportedError'], [1, 128, 768001, 'NotSupportedError'],
    [1, 128, NaN, 'TypeError'], [1, 128, Infinity, 'TypeError']
  ]) throws('constructor ' + [channels, length, rate], () => new OfflineAudioContext(channels, length, rate), error);
  const converted = new OfflineAudioContext(2 ** 32 + 1, 2 ** 32 + 128, 8192);
  check('unsigned long conversion', converted.length === 128 && converted.sampleRate === 8192);
  const context = new OfflineAudioContext(1, 128, 8192);
  const clock = Object.getOwnPropertyDescriptor(BaseAudioContext.prototype, 'currentTime');
  check('currentTime prototype getter', typeof clock?.get === 'function' && !clock.set && clock.enumerable && clock.configurable);
  check('currentTime readonly', !Object.hasOwn(context, 'currentTime') && !Reflect.set(context, 'currentTime', 100) && context.currentTime === 0);
  if (clock?.get) {
    for (const receiver of [{}, BaseAudioContext.prototype, Object.create(context), new Proxy(context, {})])
      throws('currentTime receiver', () => clock.get.call(receiver), 'TypeError');
  }
  const oscillator = context.createOscillator();
  for (const parameter of [oscillator.frequency, oscillator.detune, context.createBiquadFilter().frequency]) {
    check('default a-rate', parameter.automationRate === 'a-rate');
    parameter.automationRate = 'k-rate';
    check('set k-rate', parameter.automationRate === 'k-rate');
    parameter.automationRate = 'a-rate';
    check('set a-rate', parameter.automationRate === 'a-rate');
  }
  const compressor = context.createDynamicsCompressor();
  for (const name of ['threshold', 'knee', 'ratio', 'attack', 'release']) {
    const parameter = compressor[name];
    check('fixed k-rate ' + name, parameter.automationRate === 'k-rate');
    for (const rate of ['k-rate', 'a-rate'])
      throws('fixed setter ' + name + ' ' + rate, () => { parameter.automationRate = rate; }, 'InvalidStateError');
    check('fixed rate unchanged', parameter.automationRate === 'k-rate');
  }
  let completionCount = 0;
  context.oncomplete = () => { completionCount++; };
  const rendering = context.startRendering();
  check('render returns promise', rendering instanceof Promise);
  check('render immediate state', context.state === 'running');
  await context.startRendering().then(() => failures.push('second rendering resolved'), error => check('second rendering rejects', error.name === 'InvalidStateError'));
  const buffer = await rendering;
  check('render clock', context.currentTime === 128 / 8192 && context.state === 'closed');
  check('completion once', completionCount === 1 && buffer.length === 128);
  await context.startRendering().then(() => failures.push('closed rendering resolved'), error => check('closed rendering rejects', error.name === 'InvalidStateError'));

  const reentrant = new OfflineAudioContext(1, 4096, 8192);
  const source = reentrant.createOscillator();
  source.frequency.value = 512;
  source.connect(reentrant.destination);
  throws('reentrant start', () => source.start({valueOf() { source.start(.25); return 0; }}), 'InvalidStateError');
  throws('repeat start before range validation', () => source.start(-1), 'InvalidStateError');
  throws('restricted time before repeat check', () => source.start(NaN), 'TypeError');
  const samples = (await reentrant.startRendering()).getChannelData(0);
  check('reentrant start keeps inner schedule', samples.subarray(0, 2048).every(x => x === 0) && samples.subarray(2048).some(x => x !== 0));

  for (const frequency of [-512, -1e-20, -Number.MIN_VALUE, 0, 512]) {
    const context = new OfflineAudioContext(1, 512, 8192);
    const source = context.createOscillator();
    source.frequency.value = frequency;
    source.connect(context.destination);
    source.start();
    const pcm = (await context.startRendering()).getChannelData(0);
    check('signed frequency ' + frequency, pcm.every((x, i) => Number.isFinite(x) && Math.abs(x - Math.sin(2 * Math.PI * frequency * i / 8192)) < .0001));
  }
  for (const rate of ['a-rate', 'k-rate']) {
    const context = new OfflineAudioContext(1, 128, 8192);
    const source = context.createOscillator();
    source.frequency.automationRate = rate;
    source.frequency.setValueAtTime(256, 0);
    source.connect(context.destination);
    source.start();
    check('scheduled intrinsic before rendering ' + rate, source.frequency.value === 440);
    const pcm = (await context.startRendering()).getChannelData(0);
    check('quantum boundary PCM ' + rate, pcm.every((x, i) => Math.abs(x - Math.sin(2 * Math.PI * 256 * i / 8192)) < .0001));
    check('first quantum intrinsic ' + rate, source.frequency.value === 256);
  }
  {
    const context = new OfflineAudioContext(1, 128, 8192);
    const source = context.createOscillator();
    const filter = context.createBiquadFilter();
    const compressor = context.createDynamicsCompressor();
    filter.frequency.setValueAtTime(1024, 0);
    compressor.threshold.setValueAtTime(-50, 0);
    source.connect(filter).connect(compressor).connect(context.destination);
    source.start();
    await context.startRendering();
    check('biquad first quantum intrinsic', filter.frequency.value === 1024);
    check('compressor first quantum intrinsic', compressor.threshold.value === -50);
  }
  const realtime = new AudioContext();
  check('realtime shared clock', !Object.hasOwn(realtime, 'currentTime') && realtime.currentTime >= 0);
  await realtime.close();
  const closedClock = realtime.currentTime;
  await Promise.resolve();
  check('closed clock stable', realtime.currentTime === closedClock);
  return failures;
})()
