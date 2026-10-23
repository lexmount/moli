function exceptionConstructorStringsProbe(realm) {
  const rows = [], failures = [];
  const check = (name, actual, expected = true) => {
    rows.push({name, actual});
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };
  const units = text => Array.from({length:text.length}, (_, index) => text.charCodeAt(index));
  const values = ['', '\0', '\ud800', '\udfff', '\ud800X\udfff', '\ud83d\udca1', 'A\0\ud800\ud834\udd1e\udfffZ'];
  const constructors = [
    ['DOMException', realm.DOMException, message => [message, 'AbortError'], 'AbortError', 20],
    ['QuotaExceededError', realm.QuotaExceededError, message => [message, {quota:12.5, requested:17}], 'QuotaExceededError', 22],
    ['WebSocketError', realm.WebSocketError, message => [message, {closeCode:3000, reason:message}], 'WebSocketError', 0],
  ];
  for (const [label, Constructor, args, name, code] of constructors) {
    for (const [index, message] of values.entries()) {
      const value = Reflect.construct(Constructor, args(message));
      check(label + ': message UTF-16 ' + index, units(value.message), units(message));
      check(label + ': inherited name/code ' + index, [value.name, value.code], [name, code]);
      if (label === 'WebSocketError') {
        check(label + ': reason USVString ' + index, units(value.reason), units(message.toWellFormed()));
      } else if (label === 'QuotaExceededError') {
        check(label + ': quota data ' + index, [value.quota, value.requested], [12.5, 17]);
      }
    }
    const Derived = class extends Constructor {};
    const value = Reflect.construct(Constructor, args(values[6]), Derived);
    check(label + ': derived construction', Object.getPrototypeOf(value) === Derived.prototype && value instanceof realm.DOMException && realm.Error.isError(value));
    check(label + ': derived message UTF-16', units(value.message), units(values[6]));
    for (const input of [null, false, 42, -0, 12n]) {
      check(label + ': primitive message ' + String(input), Reflect.construct(Constructor, [input]).message, String(input));
    }
    check(label + ': missing message default', Reflect.construct(Constructor, []).message, '');
    check(label + ': undefined message default', Reflect.construct(Constructor, [undefined]).message, '');
    check(label + ': data construction has no own stack', Object.hasOwn(Reflect.construct(Constructor, []), 'stack'), false);
  }
  for (const [index, name] of values.entries()) {
    const value = new realm.DOMException('message', name);
    check('DOMException: name UTF-16 ' + index, units(value.name), units(name));
    check('DOMException: nonlegacy name code ' + index, value.code, 0);
  }
  for (const [name, code] of [['AbortError',20], ['SyntaxError',12], ['NotSupportedError',9], ['QuotaExceededError',22], ['AbOrtError',0], ['AbortError\0',0], ['AbortError\ud800',0]]) {
    check('DOMException: exact legacy code ' + JSON.stringify(name), new realm.DOMException('', name).code, code);
  }
  check('DOMException: default name', new realm.DOMException().name, 'Error');
  check('DOMException: undefined name default', new realm.DOMException('', undefined).name, 'Error');
  check('DOMException: null name string', new realm.DOMException('', null).name, 'null');
  const sentinel = {};
  const throwsSame = (name, invoke, log, expectedLog) => {
    let caught;
    try { invoke(); } catch (error) { caught = error; }
    check(name + ': exact thrown value', caught === sentinel);
    check(name + ': conversion order', log, expectedLog);
  };
  const throwsType = (name, invoke, log, expectedLog) => {
    let caught;
    try { invoke(); } catch (error) { caught = error; }
    check(name + ': callee TypeError', caught instanceof realm.TypeError);
    if (log) check(name + ': conversion order', log, expectedLog);
  };
  for (const [label, Constructor, makeInit] of [
    ['DOMException', realm.DOMException, log => ({toString(){log.push('name'); return 'AbortError';}})],
    ['QuotaExceededError', realm.QuotaExceededError, log => ({get quota(){log.push('quota');return 1;},get requested(){log.push('requested');return 2;}})],
    ['WebSocketError', realm.WebSocketError, log => ({get closeCode(){log.push('closeCode');return 3000;},get reason(){log.push('reason');return '';}})],
  ]) {
    const log = [];
    const message = {toString(){log.push('message');throw sentinel;}};
    throwsSame(label + ': message failure stops later arguments', () => Reflect.construct(Constructor,[message,makeInit(log)]), log, ['message']);
    const symbolLog = [];
    throwsType(label + ': symbol message stops later arguments', () => Reflect.construct(Constructor,[Symbol(),makeInit(symbolLog)]), symbolLog, []);
  }
  {
    const log = [];
    const value = new realm.DOMException({[Symbol.toPrimitive](hint){log.push('message:'+hint);return values[6];}},{[Symbol.toPrimitive](hint){log.push('name:'+hint);return values[4];}});
    check('DOMException: ToPrimitive order', log, ['message:string','name:string']);
    check('DOMException: converted message and name', [units(value.message),units(value.name)], [units(values[6]),units(values[4])]);
    const failed = [];
    throwsSame('DOMException: name conversion exception', () => new realm.DOMException({toString(){failed.push('message');return '';}},{toString(){failed.push('name');throw sentinel;}}), failed, ['message','name']);
  }
  {
    const log = [];
    const value = new realm.QuotaExceededError({toString(){log.push('message');return values[6];}},{get requested(){log.push('requested');return 2;},get quota(){log.push('quota');return 1;}});
    check('QuotaExceededError: lexical dictionary order after message', log, ['message','quota','requested']);
    check('QuotaExceededError: converted message preserved', units(value.message), units(values[6]));
    const failed=[];
    throwsSame('QuotaExceededError: quota getter exception stops requested', () => new realm.QuotaExceededError('', {get quota(){failed.push('quota');throw sentinel;},get requested(){failed.push('requested');return 1;}}), failed, ['quota']);
  }
  for (const input of [undefined, null]) {
    const value = new realm.WebSocketError('', input);
    check('WebSocketError: empty dictionary ' + String(input), [value.closeCode,value.reason], [null,'']);
  }
  for (const input of [false, true, 0, 42, 12n, 'text', Symbol()]) {
    const log=[];
    throwsType('WebSocketError: primitive dictionary ' + typeof input + ' ' + String(input), () => new realm.WebSocketError({toString(){log.push('message');return '';}},input), log, ['message']);
  }
  for (const [input, expected] of [[undefined,null], [3000.5,3000], [3001.5,3001], [999.5,{thrown:'InvalidAccessError'}], [1000.5,1000], ['3333',3333]]) {
    let actual;
    try { actual = new realm.WebSocketError('', {closeCode:input}).closeCode; }
    catch (error) { actual = {thrown:error.name}; }
    check('WebSocketError: closeCode conversion ' + String(input), actual, expected);
  }
  for (const input of [null, 0, 999, 1001, 2999, 5000, NaN, Infinity, -Infinity, -1, 68536, -62536, 4294970296]) {
    let caught;
    try { new realm.WebSocketError('', {closeCode:input}); } catch (error) { caught=error; }
    check('WebSocketError: invalid closeCode ' + String(input), caught instanceof realm.DOMException && caught.name === 'InvalidAccessError');
  }
  for (const input of [null, undefined, false, 42, 12n]) {
    const value = new realm.WebSocketError('', {reason:input});
    const expected=input===undefined?'':String(input);
    check('WebSocketError: reason conversion ' + String(input), [value.reason,value.closeCode], [expected,expected===''?null:1000]);
  }
  {
    const log=[];
    const init=new Proxy({closeCode:3000,reason:values[6]}, {get(target,key,receiver){log.push(String(key));return Reflect.get(target,key,receiver);},ownKeys(){throw sentinel;}});
    const value=new realm.WebSocketError({[Symbol.toPrimitive](hint){log.push('message:'+hint);return values[4];}},init);
    check('WebSocketError: dictionary Get order',log,['message:string','closeCode','reason']);
    check('WebSocketError: separate DOMString and USVString', [units(value.message),units(value.reason)], [units(values[4]),units(values[6].toWellFormed())]);
    const inherited=Object.create({closeCode:3456,reason:'inherited'});
    const fromInherited=new realm.WebSocketError('',inherited);
    check('WebSocketError: inherited dictionary members', [fromInherited.closeCode,fromInherited.reason], [3456,'inherited']);
  }
  for (const stage of ['code getter','code conversion','reason getter','reason conversion','validation after reason']) {
    const log=[];
    const init={
      get closeCode(){log.push('closeCode');if(stage==='code getter')throw sentinel;if(stage==='code conversion')return {valueOf(){log.push('code.valueOf');throw sentinel;}};return stage==='validation after reason'?999:3000;},
      get reason(){log.push('reason');if(stage==='reason getter'||stage==='validation after reason')throw sentinel;return {toString(){log.push('reason.toString');throw sentinel;}};}
    };
    const expected=stage==='code getter'?['closeCode']:stage==='code conversion'?['closeCode','code.valueOf']:stage==='reason conversion'?['closeCode','reason','reason.toString']:['closeCode','reason'];
    throwsSame('WebSocketError: '+stage,()=>new realm.WebSocketError('',init),log,expected);
  }
  {
    const log=[];
    throwsType('WebSocketError: symbol closeCode stops reason',()=>new realm.WebSocketError('',{get closeCode(){log.push('closeCode');return Symbol();},get reason(){log.push('reason');return '';}}),log,['closeCode']);
    throwsType('WebSocketError: symbol reason',()=>new realm.WebSocketError('',{reason:Symbol()}));
    const allowed=new realm.WebSocketError('',{reason:'x'.repeat(123)});
    check('WebSocketError: 123 UTF-8 bytes allowed',allowed.reason.length,123);
    let caught;
    try{new realm.WebSocketError('',{reason:'x'.repeat(124)});}catch(error){caught=error;}
    check('WebSocketError: 124 UTF-8 bytes rejected',caught instanceof realm.DOMException && caught.name==='SyntaxError');
  }
  if (realm.document) {
    for (const [index,message] of values.entries()) {
      const rtc=new realm.RTCError({errorDetail:'dtls-failure'},message);
      const constraint=new realm.OverconstrainedError(message,message);
      check('RTCError: existing UTF-16 control '+index,units(rtc.message),units(message));
      check('OverconstrainedError: existing UTF-16 control '+index,[units(constraint.message),units(constraint.constraint)],[units(message),units(message)]);
    }
  }
  return {rows,failures};
}
