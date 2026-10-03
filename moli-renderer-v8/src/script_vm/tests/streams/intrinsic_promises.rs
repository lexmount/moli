use super::*;

#[test]
fn writable_stream_writer_promises_survive_missing_or_poisoned_global_promise() {
    for (label, replacement) in [
        ("missing", "delete globalThis.Promise;"),
        ("undefined", "globalThis.Promise = undefined;"),
        (
            "throwing getter",
            r#"Object.defineProperty(globalThis, 'Promise', {
  configurable: true,
  get() {
    globalThis.__promiseHooks++;
    throw new Error('public Promise getter must not run');
  }
});"#,
        ),
        (
            "throwing constructor",
            r#"globalThis.Promise = function() {
  globalThis.__promiseHooks++;
  throw new Error('public Promise constructor must not run');
};"#,
        ),
        (
            "constructor returning a non-promise",
            r#"globalThis.Promise = function() {
  globalThis.__promiseHooks++;
  return {};
};"#,
        ),
    ] {
        let mut vm = stream_test_vm();
        vm.eval("globalThis.__intrinsicPromise = Promise; globalThis.__promiseHooks = 0;")
            .expect("capture the realm's intrinsic Promise");
        vm.eval(replacement)
            .expect("remove or poison the public Promise binding");
        vm.eval(
            r#"
globalThis.__writerIntrinsicPromiseResult = 'pending';
(async () => {
  const IntrinsicPromise = globalThis.__intrinsicPromise;
  const check = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const events = [];
  let releaseWrite;
  let reportWriteStarted;
  const writeStarted = new IntrinsicPromise(resolve => { reportWriteStarted = resolve; });
  const stream = new WritableStream({
    write(chunk) {
      events.push('write:' + chunk);
      reportWriteStarted();
      return new IntrinsicPromise(resolve => { releaseWrite = resolve; });
    },
    close() { events.push('close'); }
  }, { highWaterMark: 1 });
  const writer = stream.getWriter();
  const initialReady = writer.ready;
  const closed = writer.closed;
  check(initialReady === writer.ready && closed === writer.closed, 'initial promise identity');
  await initialReady;

  const write = writer.write('chunk');
  const pressuredReady = writer.ready;
  check(pressuredReady !== initialReady, 'backpressure replaces fulfilled ready');
  check(pressuredReady === writer.ready, 'pending ready identity');
  let readySettled = false;
  pressuredReady.then(() => { readySettled = true; });
  await writeStarted;
  check(!readySettled, 'ready stays pending while the sink write is pending');
  releaseWrite();
  await IntrinsicPromise.all([write, pressuredReady]);
  check(readySettled && writer.desiredSize === 1, 'write restores capacity and fulfills ready');

  const close = writer.close();
  await IntrinsicPromise.all([close, closed]);
  const readyBeforeRelease = writer.ready;
  writer.releaseLock();
  const releasedReady = writer.ready;
  const releasedClosed = writer.closed;
  check(releasedReady !== readyBeforeRelease && releasedClosed !== closed, 'release replaces fulfilled promises');
  const releaseErrors = await IntrinsicPromise.all([
    releasedReady.then(() => null, error => error),
    releasedClosed.then(() => null, error => error)
  ]);
  check(releaseErrors[0] instanceof TypeError && releaseErrors[0] === releaseErrors[1], 'release rejects with the same TypeError');
  check(!stream.locked, 'release unlocks the stream');
  check(
    [initialReady, closed, write, pressuredReady, close, releasedReady, releasedClosed]
      .every(promise => Object.getPrototypeOf(promise) === IntrinsicPromise.prototype),
    'all writer promises belong to the intrinsic Promise realm'
  );
  return JSON.stringify({ hooks: globalThis.__promiseHooks, events });
})().then(
  result => { globalThis.__writerIntrinsicPromiseResult = result; },
  error => { globalThis.__writerIntrinsicPromiseResult = 'error:' + error.name + ':' + error.message; }
);
"#,
        )
        .expect("writer creation and operation must not invoke the public Promise");
        let mut result = String::new();
        for _ in 0..16 {
            result = vm
                .eval("globalThis.__writerIntrinsicPromiseResult")
                .expect("writer promises must continue settling with a poisoned global Promise");
            if result != "pending" {
                break;
            }
        }
        assert_eq!(
            result, r#"{"hooks":0,"events":["write:chunk","close"]}"#,
            "global Promise case: {label}"
        );
    }
}
