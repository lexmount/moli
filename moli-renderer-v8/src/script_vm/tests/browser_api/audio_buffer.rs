use super::*;

#[test]
fn audio_buffer_public_api_matches_chromium_fixture() {
    let mut vm = new_storage_test_vm("https://audio-buffer.test/");
    let source = include_str!("../../../../tests/fixtures/audio-buffer.js");
    let result = vm
        .eval(&format!("JSON.stringify({source})"))
        .expect("AudioBuffer fixture");
    let result: serde_json::Value = serde_json::from_str(&result).expect("fixture JSON");
    assert_eq!(
        result,
        serde_json::json!({
            "construction": "ok",
            "metadata": "ok",
            "channels": "ok",
            "copy": "ok",
            "numeric_conversion": "ok",
            "bitwise_storage": "ok",
            "receivers_and_argument_order": "ok",
            "detached_views": "ok",
            "context_factory": "ok",
        })
    );
}

#[test]
fn audio_buffer_offline_result_shares_channel_views_and_copy_storage() {
    let mut vm = new_storage_test_vm("https://audio-buffer-render.test/");
    vm.exec(r#"
globalThis.bufferResults = [];
for (const connected of [false, true]) {
  const context = new OfflineAudioContext(2, 32, 8000);
  if (connected) {
    const oscillator = context.createOscillator();
    oscillator.connect(context.destination);
    oscillator.start();
  }
  context.startRendering().then(buffer => {
    const channels = [buffer.getChannelData(0), buffer.getChannelData(1)];
    const initialCopiesMatch = channels.every((channel, index) => {
      const copy = new Float32Array(32);
      buffer.copyFromChannel(copy, index);
      return copy.every((value, i) => value === channel[i]);
    });
    const initiallySilent = channels.every(channel => channel.every(x => x === 0));
    const oldSecondValue = channels[1][1];
    buffer.copyToChannel(new Float32Array([10, 20]), 0, 1);
    const copy = new Float32Array(3).fill(99);
    buffer.copyFromChannel(copy, 0, 30);
    bufferResults.push({
      connected,
      metadata: [buffer.numberOfChannels, buffer.length, buffer.sampleRate, buffer.duration],
      prototype: Object.getPrototypeOf(buffer) === AudioBuffer.prototype,
      initialCopiesMatch,
      initiallySilent,
      sharedView: buffer.getChannelData(0) === channels[0] && channels[0][1] === 10 && channels[0][2] === 20,
      independentChannels: channels[1][1] === oldSecondValue,
      untouchedSuffix: copy[2] === 99
    });
  });
}
"#, None).expect("render AudioBuffer outputs");
    let result = vm
        .eval("JSON.stringify(bufferResults)")
        .expect("completed render results");
    let result: serde_json::Value = serde_json::from_str(&result).expect("render JSON");
    assert_eq!(
        result,
        serde_json::json!([
            {"connected": false, "metadata": [2,32,8000,0.004], "prototype": true,
             "initialCopiesMatch": true, "initiallySilent": true, "sharedView": true,
             "independentChannels": true, "untouchedSuffix": true},
            {"connected": true, "metadata": [2,32,8000,0.004], "prototype": true,
             "initialCopiesMatch": true, "initiallySilent": false, "sharedView": true,
             "independentChannels": true, "untouchedSuffix": true}
        ])
    );
}
