use super::*;

#[test]
fn compression_streams_have_branded_endpoints_and_validate_formats() {
    let mut vm = stream_test_vm();
    let result = vm.eval(r#"
JSON.stringify([CompressionStream, DecompressionStream].map(C => {
  const stream = new C('gzip');
  const other = new (C === CompressionStream ? DecompressionStream : CompressionStream)('gzip');
  const throws = fn => { try { fn(); return false; } catch (e) { return e instanceof TypeError; } };
  return {
    length: C.length,
    own: Object.getOwnPropertyNames(stream),
    endpoints: stream.readable instanceof ReadableStream && stream.writable instanceof WritableStream,
    same: stream.readable === stream.readable && stream.writable === stream.writable,
    newRequired: throws(() => C('gzip')),
    notTransform: throws(() => Object.getOwnPropertyDescriptor(TransformStream.prototype, 'readable').get.call(stream)),
    formats: [undefined, '', 'GZIP', 'gzip ', 'zip', 'br', 'BROTLI', 'brotli ', Symbol()].every(f => throws(() => new C(f))),
    receiver: ['readable','writable'].every(key => {
      const get = Object.getOwnPropertyDescriptor(C.prototype, key).get;
      return [C.prototype, {}, Object.create(C.prototype), other, new TransformStream()]
        .every(receiver => throws(() => get.call(receiver)));
    })
  };
}))
"#).unwrap();
    let expected = serde_json::json!({
        "length": 1, "own": [], "endpoints": true, "same": true,
        "newRequired": true, "notTransform": true, "formats": true, "receiver": true
    });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected])
    );
}

#[test]
fn compression_streams_roundtrip_through_response_and_multiple_buffer_sources() {
    let mut vm = stream_test_vm();
    vm.eval(r#"
globalThis.compressionResult = 'pending';
(async () => {
  const result = [];
  for (const format of ['deflate-raw','deflate','gzip','brotli']) {
    const source = new ReadableStream({start(c) {
      c.enqueue(new Uint8Array([65,66]).buffer);
      c.enqueue(new DataView(new Uint8Array([0,67,68,0]).buffer, 1, 2));
      c.enqueue(new Uint8Array([0,69,70,0]).subarray(1,3));
      c.enqueue(new Uint8Array(0));
      c.close();
    }});
    const compressed = await new Response(source.pipeThrough(new CompressionStream(format))).arrayBuffer();
    const bytes = new Uint8Array(compressed);
    const split = new ReadableStream({start(c) {
      for (const byte of bytes) c.enqueue(new Uint8Array([byte]));
      c.close();
    }});
    const text = await new Response(split.pipeThrough(new DecompressionStream(format))).text();
    const empty = new ReadableStream({start(c) { c.close(); }});
    const emptyResult = await new Response(empty.pipeThrough(new CompressionStream(format))
      .pipeThrough(new DecompressionStream(format))).text();
    result.push([text, emptyResult, bytes.length > 0]);
  }
  return result;
})().then(result => compressionResult = result, e => compressionResult = [e.name, e.message]);
"#).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(compressionResult)").unwrap(),
        r#"[["ABCDEF","",true],["ABCDEF","",true],["ABCDEF","",true],["ABCDEF","",true]]"#
    );
}

#[test]
fn compression_streams_reject_invalid_chunks_and_corrupt_or_truncated_data() {
    let mut vm = stream_test_vm();
    vm.eval(r#"
globalThis.compressionErrors = 'pending';
(async () => {
  const rejects = promise => promise.then(() => 'resolved', e => e.name);
  const results = [];
  for (const C of [CompressionStream, DecompressionStream]) {
    for (const chunk of ['bytes', {}, 1, null]) {
      const s = new C('gzip');
      const writer = s.writable.getWriter();
      const reader = s.readable.getReader();
      results.push(await Promise.all([rejects(reader.read()), rejects(writer.write(chunk))]));
    }
  }
  for (const format of ['deflate-raw','deflate','gzip','brotli']) {
    const source = new ReadableStream({start(c) { c.enqueue(new Uint8Array([65])); c.close(); }});
    const bytes = new Uint8Array(await new Response(source.pipeThrough(new CompressionStream(format))).arrayBuffer());
    for (const bad of [bytes.slice(0,-1), new Uint8Array([...bytes,0]), new Uint8Array([255,255])]) {
      const source = new ReadableStream({start(c) { c.enqueue(bad); c.close(); }});
      results.push(await rejects(new Response(source.pipeThrough(new DecompressionStream(format))).arrayBuffer()));
    }
  }
  return results;
})().then(result => compressionErrors = result, e => compressionErrors = [e.name,e.message]);
"#).unwrap();
    let result = vm.eval("JSON.stringify(compressionErrors)").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    let result = result.as_array().unwrap();
    assert_eq!(result.len(), 20);
    for value in &result[..8] {
        assert_eq!(*value, serde_json::json!(["TypeError", "TypeError"]));
    }
    for value in &result[8..] {
        assert_eq!(*value, "TypeError");
    }
}

#[test]
fn compression_streams_preserve_backpressure_and_cancel_or_abort_reasons() {
    let mut vm = stream_test_vm();
    vm.eval(
        r#"
globalThis.compressionEvents = [];
globalThis.compressor = new CompressionStream('gzip');
globalThis.compressionWriter = compressor.writable.getWriter();
compressionWriter.write(new Uint8Array([65])).then(() => compressionEvents.push('write'));
"#,
    )
    .unwrap();
    assert_eq!(vm.eval("JSON.stringify(compressionEvents)").unwrap(), "[]");
    vm.eval(r#"
compressor.readable.getReader().read().then(({value,done}) => compressionEvents.push(`${value instanceof Uint8Array}:${done}`));
"#).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(compressionEvents.sort())").unwrap(),
        r#"["true:false","write"]"#
    );
    vm.eval(
        r#"
globalThis.compressionCancellation = 'pending';
(async () => {
  const reason = {};
  const result = [];
  for (const format of ['deflate-raw','deflate','gzip','brotli']) {
    for (const C of [CompressionStream, DecompressionStream]) {
      const s = new C(format);
      const writer = s.writable.getWriter();
      const closed = writer.closed.catch(e => e === reason);
      const write = writer.write(new Uint8Array([1])).catch(e => e === reason);
      await s.readable.cancel(reason);
      result.push(await closed, await write);
      const aborted = new C(format);
      const reader = aborted.readable.getReader();
      const read = reader.read().catch(e => e === reason);
      await aborted.writable.abort(reason);
      result.push(await read);
    }
  }
  return result;
})().then(result => compressionCancellation = result, e => compressionCancellation = e.message);
"#,
    )
    .unwrap();
    let result = vm.eval("JSON.stringify(compressionCancellation)").unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<bool>>(&result).unwrap(),
        vec![true; 24]
    );
}

#[test]
fn brotli_compression_preserves_backpressure_and_flushes_on_close() {
    let mut vm = stream_test_vm();
    vm.eval(
        r#"
globalThis.brotliEvents = [];
globalThis.brotliStream = new CompressionStream('brotli');
globalThis.brotliWriter = brotliStream.writable.getWriter();
brotliWriter.write(new Uint8Array([65,66])).then(() => brotliEvents.push('write'));
"#,
    )
    .unwrap();
    assert_eq!(vm.eval("JSON.stringify(brotliEvents)").unwrap(), "[]");
    vm.eval(
        r#"
globalThis.brotliBytes = new Response(brotliStream.readable).arrayBuffer();
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(brotliEvents)").unwrap(),
        r#"["write"]"#
    );
    vm.eval(
        r#"
globalThis.brotliResult = 'pending';
(async () => {
  await brotliWriter.close();
  const bytes = await brotliBytes;
  const source = new ReadableStream({start(c) { c.enqueue(bytes); c.close(); }});
  return await new Response(source.pipeThrough(new DecompressionStream('brotli'))).text();
})().then(value => brotliResult = value, error => brotliResult = error.message);
"#,
    )
    .unwrap();
    assert_eq!(vm.eval("brotliResult").unwrap(), "AB");
}
