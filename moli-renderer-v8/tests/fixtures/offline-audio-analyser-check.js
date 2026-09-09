(async () => {
  const results = [];
  for (const frequency of [256, 512, 1024]) {
    const rate = 8192, length = 4096, fftSize = 256;
    const context = new OfflineAudioContext(1, length, rate);
    const oscillator = context.createOscillator();
    oscillator.frequency.value = frequency;
    const analyser = context.createAnalyser();
    analyser.fftSize = fftSize;
    analyser.smoothingTimeConstant = 0;
    analyser.minDecibels = -60;
    analyser.maxDecibels = 0;
    oscillator.connect(analyser);
    analyser.connect(context.destination);
    oscillator.start();
    const buffer = await context.startRendering();
    const pcm = buffer.getChannelData(0);
    const time = new Float32Array(fftSize);
    analyser.getFloatTimeDomainData(time);
    const bins = new Float32Array(fftSize / 2);
    analyser.getFloatFrequencyData(bins);
    const again = new Float32Array(bins.length);
    analyser.getFloatFrequencyData(again);
    const bytes = new Uint8Array(bins.length);
    analyser.getByteFrequencyData(bytes);
    const byteTime = new Uint8Array(fftSize);
    analyser.getByteTimeDomainData(byteTime);
    let peak = 0;
    for (let i = 1; i < bins.length; ++i) if (bins[i] > bins[peak]) peak = i;
    const waveformError = Math.max(...pcm.map((value, i) => Math.abs(value - Math.sin(2 * Math.PI * frequency * i / rate))));
    const retained = Array.from(time);
    oscillator.disconnect();
    analyser.disconnect();
    analyser.getFloatTimeDomainData(time);
    results.push({
      frequency,
      waveformError,
      waveformCorrect: waveformError < 0.0001,
      timeMatchesPcm: time.every((value, i) => value === pcm[length - fftSize + i]),
      retainsAfterDisconnect: time.every((value, i) => value === retained[i]),
      peak,
      peakCorrect: peak === frequency * fftSize / rate,
      peakDecibels: bins[peak],
      blackmanAmplitudeCorrect: Math.abs(bins[peak] - 20 * Math.log10(0.21)) < 0.001,
      frequencyReadStable: bins.every((value, i) => value === again[i]),
      byteFrequencyCorrect: bytes.every((value, i) => value === Math.max(0, Math.min(255, Math.floor(255 * (bins[i] + 60) / 60)))),
      byteTimeCorrect: byteTime.every((value, i) => value === Math.max(0, Math.min(255, Math.floor(128 * (time[i] + 1))))),
    });
  }
  return results;
})()
