async function runFetchBodyNativeProbe(scenario) {
  const errors = [];
  const check = (condition, message) => { if (!condition) errors.push(message); };
  const encode = value => new TextEncoder().encode(value);
  const own = (value, name) => Object.prototype.hasOwnProperty.call(value, name);
  const closed = chunk => new ReadableStream({ start(c) { c.enqueue(chunk); c.close(); } });
  const makeBody = (type, stream, mime = "text/plain") => type === "Response"
    ? new Response(stream, { headers: { "Content-Type": mime } })
    : new Request("https://example.test/body", {
      method: "POST", body: stream, duplex: "half", headers: { "Content-Type": mime }
    });
  const rejectWith = async (promise, predicate, name) => {
    try { await promise; errors.push(name + " resolved"); }
    catch (error) { check(predicate(error), name + " wrong rejection: " + String(error)); }
  };

  if (scenario === "then") {
    for (const type of ["Response", "Request"]) {
      for (const injected of [{ done: false, value: encode("bye") },
        { done: false, value: undefined }, undefined, 8.2]) {
        const body = makeBody(type, closed(encode("hello")));
        let calls = 0;
        Object.prototype.then = resolve => {
          ++calls;
          delete Object.prototype.then;
          resolve(injected);
        };
        try {
          check(await body.text() === "hello", type + " injected read result");
          check(calls === 0, type + " exposed internal thenable");
        } catch (error) {
          errors.push(type + " inherited then: " + String(error));
        } finally { delete Object.prototype.then; }
      }
      for (const method of ["text", "json", "bytes", "arrayBuffer", "blob", "formData"]) {
        const payload = method === "json" ? '{"answer":42}' : method === "formData" ? "answer=42" : "hello";
        const body = makeBody(type, closed(encode(payload)), "application/x-www-form-urlencoded");
        let reads = 0;
        let values = 0;
        Object.defineProperty(Object.prototype, "then", { configurable: true, get() {
          if (own(this, "done") && own(this, "value")) { ++reads; throw new Error("internal read leaked"); }
          if (!(this instanceof Promise)) ++values;
          return undefined;
        } });
        let value;
        try { value = await body[method](); }
        catch (error) { errors.push(type + "." + method + " then getter: " + String(error)); }
        finally { delete Object.prototype.then; }
        check(reads === 0, type + "." + method + " exposed read result");
        check(values === (method === "text" ? 0 : 1), type + "." + method + " result assimilation count " + values);
        if (method === "text") check(value === payload, type + " text bytes");
        if (method === "json") check(value?.answer === 42, type + " json bytes");
        if (method === "bytes") check(value instanceof Uint8Array && new TextDecoder().decode(value) === payload, type + " bytes value");
        if (method === "arrayBuffer") check(value instanceof ArrayBuffer && new TextDecoder().decode(value) === payload, type + " arrayBuffer value");
        if (method === "blob") check(value instanceof Blob && await value.text() === payload, type + " blob value");
        if (method === "formData") check(value instanceof FormData && value.get("answer") === "42", type + " formData value");
      }
    }
    // Public reader results still undergo the Promise resolution procedure.
    const reader = closed(encode("hello")).getReader();
    const replacement = { public: true };
    Object.prototype.then = resolve => { delete Object.prototype.then; resolve(replacement); };
    try { check(await reader.read() === replacement, "public reader thenable semantics changed"); }
    finally { delete Object.prototype.then; }
  } else if (scenario === "chunks") {
    for (const type of ["Response", "Request"]) {
      const chunk = encode("_hello_").subarray(1, 6);
      let getters = 0;
      for (const key of [Symbol.toStringTag, "byteLength", "constructor"]) {
        Object.defineProperty(chunk, key, { get() { ++getters; throw new Error("chunk getter"); } });
      }
      try { check(await makeBody(type, closed(chunk)).text() === "hello", type + " offset bytes"); }
      catch (error) { errors.push(type + " valid chunk: " + String(error)); }
      check(getters === 0, type + " read chunk properties");
      for (const invalid of [undefined, [], new Uint8ClampedArray([1]),
        new DataView(new ArrayBuffer(1)), new Proxy(encode("x"), {}),
        { [Symbol.toStringTag]: "Uint8Array", byteLength: 1, 0: 120 }]) {
        await rejectWith(makeBody(type, closed(invalid)).text(), e => e instanceof TypeError, type + " invalid chunk");
      }
    }
    for (const phase of ["pull", "enqueue"]) {
      const chunk = encode("hello");
      let pulled = false;
      const source = new ReadableStream({
        start(c) { if (phase === "pull") c.enqueue(chunk); },
        pull(c) {
          if (pulled) return;
          pulled = true;
          if (phase === "enqueue") c.enqueue(chunk);
          chunk.fill(120);
          c.close();
        }
      }, { highWaterMark: 0 });
      check(await new Response(source).text() === "hello", "chunk copy before " + phase + " mutation");
    }
    const chunk = encode("hello");
    let controller;
    const response = new Response(new ReadableStream({ start(c) { controller = c; } }));
    const pending = response.text();
    controller.enqueue(chunk);
    structuredClone(chunk.buffer, { transfer: [chunk.buffer] });
    controller.close();
    try { check(await pending === "hello", "chunk copy before detachment"); }
    catch (error) { errors.push("detached after enqueue: " + String(error)); }
    const count = 12000;
    const many = new ReadableStream({ start(c) {
      for (let i = 0; i < count; ++i) c.enqueue(new Uint8Array([i % 251]));
      c.close();
    } });
    const bytes = await new Response(many).bytes();
    check(bytes.length === count && bytes.every((value, index) => value === index % 251), "queued chunk order or stack depth");
    const byob = new ReadableStream({ type: "bytes", autoAllocateChunkSize: 8, pull(c) {
      c.byobRequest.view.set(encode("hello"));
      c.byobRequest.respond(5);
      c.close();
    } });
    check(await new Response(byob).text() === "hello", "auto allocated byte reader");
  } else if (scenario === "errors") {
    for (const type of ["Response", "Request"]) {
      for (const reason of [undefined, Symbol("reason"), { error: "original" }]) {
        for (const phase of ["start", "pull", "after-chunk"]) {
          const stream = new ReadableStream({
            start(c) { if (phase === "start") c.error(reason); },
            pull(c) {
              if (phase === "after-chunk") c.enqueue(encode("partial"));
              c.error(reason);
            }
          });
          const body = makeBody(type, stream);
          await rejectWith(body.text(), e => e === reason, type + " " + phase);
          check(body.bodyUsed && body.body.locked, type + " error consumption access state");
        }
      }
      await rejectWith(makeBody(type, closed(encode("{"))).json(), e => e instanceof SyntaxError, type + " JSON parse error");
      await rejectWith(makeBody(type, closed(encode("hello"))).formData(), e => e instanceof TypeError, type + " invalid form MIME");
      const body = makeBody(type, closed(encode("hello")));
      const first = body.text();
      check(body.bodyUsed && body.body.locked, type + " synchronous consumption access state");
      await rejectWith(body.text(), e => e instanceof TypeError, type + " repeated consumption");
      check(await first === "hello", type + " first consumption");
    }
  } else if (scenario === "intrinsics") {
    for (const method of ["text", "json", "bytes", "arrayBuffer", "blob", "formData"]) {
      const payload = method === "json" ? '{"answer":42}' : method === "formData" ? "answer=42" : "hello";
      const body = makeBody("Response", closed(encode(payload)), "application/x-www-form-urlencoded");
      const saved = Object.fromEntries(["TextDecoder", "Uint8Array", "Blob", "Response"].map(key => [key, globalThis[key]]));
      const parse = JSON.parse;
      let value;
      const poison = () => { throw new Error("public intrinsic called"); };
      try {
        for (const key of Object.keys(saved)) globalThis[key] = poison;
        JSON.parse = poison;
        value = await body[method]();
      } catch (error) { errors.push(method + " public constructor: " + String(error)); }
      finally { Object.assign(globalThis, saved); JSON.parse = parse; }
      if (method === "text") check(value === payload, "native text");
      if (method === "json") check(value?.answer === 42, "native JSON");
      if (method === "bytes") check(value instanceof Uint8Array, "native bytes");
      if (method === "arrayBuffer") check(value instanceof ArrayBuffer, "native arrayBuffer");
      if (method === "blob") check(value instanceof Blob && await value.text() === payload, "native blob");
      if (method === "formData") check(value instanceof FormData && value.get("answer") === "42", "native formData");
    }
  }
  return { errors };
}
