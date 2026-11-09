(async () => {
  const rows = [];
  const assert = (condition, message) => { if (!condition) throw Error(message); };
  const check = async (name, callback) => {
    try { await callback(); rows.push({name, pass: true}); }
    catch (error) { rows.push({name, pass: false, message: String(error)}); }
  };
  const rejected = async context => {
    let error; try { await context.startRendering(); } catch (value) { error = value; }
    assert(error instanceof DOMException && error.name === 'NotSupportedError', 'active channel routing requires a real processing backend');
    assert(context.state === 'suspended', 'failure does not commit rendering state');
  };
  for (const factory of ['createChannelMerger', 'createChannelSplitter']) {
    for (const automaticPull of [false, true]) {
      await check(factory + '/active input/' + automaticPull, async () => {
        const context = new OfflineAudioContext(1, 32, 8000), source = context.createOscillator(), node = context[factory](3), analyser = context.createAnalyser();
        source.connect(node); node.connect(analyser, factory === 'createChannelSplitter' ? 2 : 0);
        if (!automaticPull) analyser.connect(context.destination);
        node.__moliAudioNodeNeedsProcessingBackend = false; source.start();
        await rejected(context);
        const bins = new Float32Array(analyser.frequencyBinCount); analyser.getFloatFrequencyData(bins);
        assert(bins.every(value => value === -Infinity), 'failure does not commit analyser snapshots');
      });
    }
    await check(factory + '/future source is silence', async () => {
      const context = new OfflineAudioContext(1, 32, 8000), source = context.createOscillator(), node = context[factory](3);
      source.start(1); source.connect(node); node.connect(context.destination);
      const buffer = await context.startRendering();
      assert(buffer.getChannelData(0).every(value => value === 0), 'provable silence does not require processing');
    });
    await check(factory + '/unreachable node preserves existing graph', async () => {
      const context = new OfflineAudioContext(1, 32, 8000), source = context.createOscillator(), node = context[factory](3);
      source.connect(node); source.connect(context.destination); source.start();
      const buffer = await context.startRendering();
      assert(buffer.getChannelData(0).some(value => value !== 0), 'disconnected router does not block another graph');
    });
  }
  await check('merger/selective input disconnect and retry', async () => {
    const context = new OfflineAudioContext(1, 32, 8000), source = context.createOscillator(), merger = context.createChannelMerger(3);
    source.connect(merger, 0, 0); source.connect(merger, 0, 1); source.connect(merger, 0, 1);
    merger.connect(context.destination); source.start(); source.disconnect(merger, 0, 0);
    await rejected(context);
    source.disconnect(merger, 0, 1);
    const buffer = await context.startRendering();
    assert(buffer.getChannelData(0).every(value => value === 0), 'removing the final deduplicated connection permits silent retry');
  });
  await check('splitter/selective output disconnect and retry', async () => {
    const context = new OfflineAudioContext(1, 32, 8000), source = context.createOscillator(), splitter = context.createChannelSplitter(3);
    source.connect(splitter); splitter.connect(context.destination, 0); splitter.connect(context.destination, 2);
    source.start(); splitter.disconnect(0);
    await rejected(context);
    splitter.disconnect(2);
    const buffer = await context.startRendering();
    assert(buffer.getChannelData(0).every(value => value === 0), 'only the requested output was disconnected');
  });
  await check('splitter/automatic pull survives another port removal', async () => {
    const context = new OfflineAudioContext(1, 32, 8000), source = context.createOscillator(), splitter = context.createChannelSplitter(3), analyser = context.createAnalyser();
    source.connect(splitter); splitter.connect(analyser, 0); splitter.connect(analyser, 2); source.start();
    splitter.disconnect(analyser, 0); await rejected(context);
    splitter.disconnect(analyser, 2);
    const buffer = await context.startRendering();
    assert(buffer.getChannelData(0).every(value => value === 0), 'automatic pull ends after the last incoming connection');
  });
  globalThis.__nodeReplacementResults = {rows, passed: rows.filter(row => row.pass).length, total: rows.length, failures: rows.filter(row => !row.pass)};
  return rows.every(row => row.pass);
})()
