(async () => {
  const rows = [], assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, run) => { try { await run(); rows.push({name, pass: true}); } catch (error) { rows.push({name, pass: false, message: String(error)}); } };
  const setup = () => {
    const context = new OfflineAudioContext(1, 16, 48000), source = context.createOscillator(), analyser = context.createAnalyser();
    source.setPeriodicWave(new PeriodicWave(context, {imag: [0, 1]}));
    source.connect(analyser); analyser.connect(context.destination);
    return {context, source, analyser};
  };
  await check('custom oscillator rejects active PCM before committing state and can retry', async () => {
    const {context, source, analyser} = setup(); source.start();
    source.__moliOscillatorType = 'sine'; Object.defineProperty(source, 'type', {value: 'sine'});
    let error; const promise = context.startRendering(); assert(promise instanceof Promise, 'Promise return');
    try { await promise; } catch (value) { error = value; }
    assert(error && error.name === 'NotSupportedError' && context.state === 'suspended', 'explicit backend rejection and no render commit');
    const data = new Float32Array(3); analyser.getFloatTimeDomainData(data); assert(data.every(value => value === 0), 'no analyser snapshot');
    Object.getOwnPropertyDescriptor(OscillatorNode.prototype, 'type').set.call(source, 'sine');
    const buffer = await context.startRendering(); assert(buffer.length === 16 && context.state === 'closed', 'native built-in reset permits retry');
  });
  await check('automatic analyser pull also rejects custom PCM', async () => {
    const {context, source, analyser} = setup(); analyser.disconnect(); source.start();
    let error; try { await context.startRendering(); } catch (value) { error = value; }
    assert(error && error.name === 'NotSupportedError' && context.state === 'suspended', 'automatic pull guarded');
  });
  await check('inactive or unreachable custom oscillators preserve silence', async () => {
    for (const mode of ['unstarted', 'future', 'stopped', 'unreachable']) {
      const {context, source, analyser} = setup();
      if (mode === 'future') source.start(1);
      if (mode === 'stopped') { source.start(); source.stop(0); }
      if (mode === 'unreachable') { source.start(); source.disconnect(); }
      const buffer = await context.startRendering();
      assert(buffer.getChannelData(0).every(value => value === 0), mode + ' silence');
      const data = new Float32Array(3); analyser.getFloatTimeDomainData(data); assert(data.every(value => value === 0), mode + ' analyser silence');
    }
  });
  const result = {total: rows.length, passed: rows.filter(row => row.pass).length, failures: rows.filter(row => !row.pass), rows};
  globalThis.__nodeReplacementResults = result;
  return result.failures.length === 0;
})()
