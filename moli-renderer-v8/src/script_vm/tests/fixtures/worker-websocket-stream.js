globalThis.workerStreamFrontendContract = async function(base) {
  let checks = 0;
  const stages = [];
  const check = (value, message) => {checks++;if (!value) throw Error(message);};
  check(typeof WebSocketStream === 'function', 'WebSocketStream constructor');
  check(WebSocketStream.name === 'WebSocketStream' && WebSocketStream.length === 1, 'constructor metadata');
  check(Function.prototype.toString.call(WebSocketStream).includes('[native code]'), 'native constructor');
  check(WebSocketStream.prototype.constructor === WebSocketStream, 'prototype constructor');
  check(probeWebSocketStreamAbortOptions(globalThis,globalThis), 'native options');
  stages.push('options');
  for (const mode of ['getter','noop','stopping','prototype']) {
    const result = await probeWebSocketStreamNativeAbort(globalThis,globalThis,base+'/handshake_sleep_2',mode);
    check(result.gets === 0 && result.calls === 0 && result.opened === 'AbortError' && result.closed === 'AbortError', 'native abort '+mode);
  }
  stages.push('abort');
  check(await probeWebSocketStreamSyntheticAbort(globalThis,globalThis,base+'/echo'), 'synthetic abort and echo');
  stages.push('synthetic');
  const NativePromise = Promise;
  const descriptor = Object.getOwnPropertyDescriptor(globalThis,'Promise');
  let gets = 0, stream;
  try {
    Object.defineProperty(globalThis,'Promise',{configurable:true,get(){gets++;throw Error('author Promise');}});
    stream = new WebSocketStream(base+'/echo');
    check(stream.opened instanceof NativePromise && stream.closed instanceof NativePromise, 'intrinsic promises');
    check(stream.opened === stream.opened && stream.closed === stream.closed, 'stable promises');
  } finally {Object.defineProperty(globalThis,'Promise',descriptor);}
  check(gets === 0, 'zero author Promise gets');
  const info = await stream.opened;
  check(info.readable instanceof ReadableStream && info.writable instanceof WritableStream, 'native streams');
  check(typeof info.protocol === 'string' && typeof info.extensions === 'string', 'open strings');
  const reader = info.readable.getReader(), writer = info.writable.getWriter();
  check(writer.desiredSize === 1, 'initial writable size');
  let conversions = 0;
  await writer.write({toString(){conversions++;return 'worker-text';}});
  const text = await reader.read();
  check(conversions === 1 && !text.done && text.value === 'worker-text', 'text conversion and echo');
  await writer.write(new Uint8Array([0,7,128,255]));
  const binary = await reader.read();
  const binaryKind = binary.value?.constructor?.name;
  check(binary.value instanceof Uint8Array || binary.value instanceof ArrayBuffer, 'native binary value');
  const binaryBytes = binary.value instanceof Uint8Array ? binary.value : new Uint8Array(binary.value);
  check(!binary.done && Array.from(binaryBytes).join() === '0,7,128,255', 'binary bytes');
  const closeValue = await writer.close();
  const closed = await stream.closed;
  check(closeValue === undefined && closed.closeCode === 1005 && closed.reason === '', 'writer clean close');
  check((await reader.read()).done, 'readable closes');
  stages.push('io');
  const protocolStream = new WebSocketStream(base+'/protocol_array',{protocols:['alpha','beta']});
  const protocolInfo = await protocolStream.opened;
  check(protocolInfo.protocol === 'alpha', 'subprotocol');
  const protocolReader = protocolInfo.readable.getReader();
  check((await protocolReader.read()).value === 'alpha', 'subprotocol response');
  protocolStream.close();
  await protocolStream.closed;
  stages.push('protocol');
  const explicitStream = new WebSocketStream(base+'/echo');
  await explicitStream.opened;
  explicitStream.close({closeCode:3456,reason:'worker done'});
  const explicitClose = await explicitStream.closed;
  check(explicitClose.closeCode === 3456 && explicitClose.reason === 'worker done', 'explicit close');
  stages.push('explicit-close');
  for (const mode of ['abort','cancel']) {
    const socket = new WebSocketStream(base+'/echo');
    const opened = await socket.opened;
    await opened[mode === 'abort' ? 'writable' : 'readable'][mode](new WebSocketError('',{closeCode:3333,reason:mode}));
    const closed = await socket.closed;
    check(closed.closeCode === 3333 && closed.reason === mode, mode+' close info');
  }
  stages.push('abort-cancel');
  return {checks,stages,binaryKind};
};
