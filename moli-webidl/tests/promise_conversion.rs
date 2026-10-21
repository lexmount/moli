use moli_webidl::{DomString, Record, WebIdlArgs};

#[derive(WebIdlArgs)]
struct PromiseArgs<'s> {
    #[webidl(required)]
    value: v8::Local<'s, v8::Promise>,
}

#[derive(WebIdlArgs)]
struct PromiseRecordArgs<'s> {
    #[webidl(required, converter = "raw")]
    values: Record<DomString, v8::Local<'s, v8::Promise>>,
}

fn convert_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<PromiseArgs>(scope, &args) {
        rv.set(parsed.value.into());
    }
}

fn convert_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<PromiseRecordArgs>(scope, &args) {
        let promises = parsed
            .values
            .0
            .iter()
            .map(|(_, promise)| (*promise).into())
            .collect::<Vec<_>>();
        rv.set(v8::Array::new_with_elements(scope, &promises).into());
    }
}

fn evaluate<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .expect("Promise conversion test script")
}

fn check_conversion(source: &str) {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let (convert, record, intrinsic, type_error) = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let convert = v8::Function::new(scope, convert_promise).unwrap();
        let record = v8::Function::new(scope, convert_record).unwrap();
        let intrinsic = evaluate(scope, "Promise");
        let type_error = evaluate(scope, "TypeError");
        evaluate(
            scope,
            "globalThis.Promise = function() { throw Error('author Promise constructor'); }",
        );
        (convert, record, intrinsic, type_error)
    };
    let caller = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    let global = caller.global(scope);
    for (name, value) in [
        ("convertPromise", convert.into()),
        ("convertRecord", record.into()),
        ("CalleePromise", intrinsic),
        ("CalleeTypeError", type_error),
    ] {
        let key = v8::String::new(scope, name).unwrap();
        assert_eq!(
            global.create_data_property(scope, key.into(), value),
            Some(true)
        );
    }
    let result = evaluate(scope, source);
    let promise = v8::Local::<v8::Promise>::try_from(result).unwrap();
    scope.perform_microtask_checkpoint();
    assert_eq!(
        promise.state(),
        v8::PromiseState::Fulfilled,
        "{}",
        promise.result(scope).to_rust_string_lossy(scope)
    );
    assert!(promise.result(scope).is_true());
}

#[test]
fn native_promises_are_wrapped_in_new_intrinsic_callee_capabilities() {
    check_conversion(
        r#"(async () => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const CallerPromise = Promise;
  for (const SourcePromise of [CallerPromise, CalleePromise]) {
    const log = [], value = {}, sentinel = {};
    let constructorReads = 0;
    const input = SourcePromise.resolve(value);
    Object.defineProperty(input, 'constructor', {get() {constructorReads++; throw sentinel;}});
    Object.defineProperty(input, 'then', {get() {
      log.push('get');
      return function(resolve) {
        assert(this === input, 'then receives the original input');
        log.push('call');
        resolve(value);
      };
    }});
    const first = convertPromise(input), second = convertPromise(input);
    assert(first !== input && second !== input && first !== second, 'fresh capabilities');
    assert(first instanceof CalleePromise && second instanceof CalleePromise && !(first instanceof CallerPromise), 'callee intrinsics');
    assert(log.join() === 'get,get' && constructorReads === 0, 'synchronous then reads without PromiseResolve constructor lookup');
    globalThis.Promise = function() {throw Error('caller public Promise');};
    await 0; await 0;
    assert(log.join() === 'get,get,call,call', 'asynchronous then calls');
    assert(await first === value && await second === value, 'resolution preserves value identity');
    assert(constructorReads === 0, 'custom then does not consult input constructor');
  }
  return true;
})()"#,
    );
}

#[test]
fn promise_conversion_returns_rejections_for_throwing_then_and_revoked_proxies() {
    check_conversion(
        r#"(async () => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const sentinel = {}, input = Promise.resolve(1);
  let reads = 0;
  Object.defineProperty(input, 'then', {get() {reads++; throw sentinel;}});
  const output = convertPromise(input);
  assert(output !== input && output instanceof CalleePromise && reads === 1, 'native then getter runs during wrapping');
  let error;
  try {await output;} catch (caught) {error = caught;}
  assert(error === sentinel, 'getter exception becomes rejection with the same value');
  const revoked = Proxy.revocable({}, {}); revoked.revoke();
  const rejected = convertPromise(revoked.proxy);
  assert(rejected instanceof CalleePromise, 'revoked proxy still produces a capability');
  try {await rejected; throw Error('expected rejection');} catch (caught) {
    assert(caught instanceof CalleeTypeError && !(caught instanceof TypeError), 'revoked proxy error realm');
  }
  for (const value of [undefined, null, true, 1, 'value', 2n, Symbol('value')]) {
    assert(await convertPromise(value) === value, 'scalar is resolved without coercion');
  }
  try {convertPromise(); throw Error('expected arity error');} catch (caught) {
    assert(caught instanceof CalleeTypeError, 'generated required argument check');
  }
  return true;
})()"#,
    );
}

#[test]
fn promise_record_conversion_wraps_each_entry_before_the_next_value_is_read() {
    check_conversion(
        r#"(async () => {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const log = [];
  const native = key => {
    const input = Promise.resolve('ignored');
    Object.defineProperty(input, 'then', {get() {
      log.push('then:' + key);
      return resolve => {log.push('call:' + key); resolve(key);};
    }});
    return input;
  };
  const second = native('second'), first = native('first');
  const outputs = convertRecord({
    get second() {log.push('value:second'); return second;},
    get first() {log.push('value:first'); return first;}
  });
  assert(log.join() === 'value:second,then:second,value:first,then:first', 'record entry conversion order');
  assert(outputs.length === 2 && outputs.every(value => value instanceof CalleePromise), 'record promise realms');
  assert(outputs[0] !== second && outputs[1] !== first, 'record entries are wrapped');
  assert(await outputs[0] === 'second' && await outputs[1] === 'first', 'entry resolution follows the supplied then');
  assert(log.join() === 'value:second,then:second,value:first,then:first,call:second,call:first', 'one asynchronous then call per entry');
  return true;
})()"#,
    );
}
