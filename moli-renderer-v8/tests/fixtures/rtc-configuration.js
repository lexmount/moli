(() => {
  const checks = [];
  const assert = (value, message = 'assertion failed') => { if (!value) throw Error(message); };
  const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])) : value;
  const equal = (value, expected) => assert(JSON.stringify(canonical(value)) === JSON.stringify(canonical(expected)), JSON.stringify({value, expected}));
  const run = (name, body) => {
    try { body(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const defaults = {bundlePolicy:'balanced', certificates:[], iceCandidatePoolSize:0, iceServers:[], iceTransportPolicy:'all', rtcpMuxPolicy:'require'};
  for (const [label, realm] of [['main', globalThis], ['iframe', document.querySelector('iframe').contentWindow]]) {
    const C = realm.RTCPeerConnection;
    const check = (name, body) => run(label + ': ' + name, body);
    const withPc = (config, body) => { const pc = new C(config); try { body(pc); } finally { pc.close(); } };
    const throws = (name, body) => {
      let error; try { body(); } catch (caught) { error = caught; }
      assert(error?.name === name, 'expected ' + name + ', got ' + String(error));
      assert(error instanceof realm[name === 'TypeError' ? 'TypeError' : 'DOMException'], 'callee error realm');
    };
    check('method descriptors', () => {
      for (const name of ['getConfiguration','setConfiguration']) {
        const d = Object.getOwnPropertyDescriptor(C.prototype, name);
        assert(d && d.writable && d.enumerable && d.configurable && d.value.length === 0);
      }
    });
    for (const [name, config] of [['omitted',undefined],['null',null],['empty',{}],['function',function(){}]]) {
      check('default ' + name, () => withPc(config, pc => {
        const c = pc.getConfiguration();
        for (const key of Object.keys(defaults)) equal(c[key], defaults[key]);
      }));
    }
    for (const value of [0, true, 'configuration', Symbol('x'), 1n]) {
      check('reject dictionary primitive ' + String(value), () => throws('TypeError', () => new C(value)));
    }
    check('constructor snapshots nested records', () => {
      const urls = ['stun:stun.example.org', 'turn:turn.example.org?transport=tcp'];
      const server = {urls, username:'user', credential:'secret'};
      const input = {iceServers:[server], iceTransportPolicy:'relay', ignored:123};
      withPc(input, pc => {
        input.iceTransportPolicy = 'all'; urls[0] = 'stun:changed.example'; server.username = 'changed'; input.iceServers.push({urls:'stun:extra.example'});
        const first = pc.getConfiguration();
        equal(first.iceTransportPolicy,'relay'); equal(first.iceServers,[{urls:['stun:stun.example.org','turn:turn.example.org?transport=tcp'],username:'user',credential:'secret'}]);
        assert(!Object.hasOwn(first, 'ignored'));
        first.iceServers[0].urls[0] = 'stun:mutated.example'; first.iceServers[0].credential = 'mutated'; first.certificates.push({}); first.iceTransportPolicy='all';
        const second = pc.getConfiguration(); assert(first !== second && first.iceServers !== second.iceServers && first.iceServers[0] !== second.iceServers[0]);
        equal(second.iceTransportPolicy,'relay'); equal(second.iceServers[0].urls[0],'stun:stun.example.org'); equal(second.iceServers[0].credential,'secret'); equal(second.certificates,[]);
      });
    });
    check('conversion getters read once in lexical order', () => {
      const seen = [];
      const value = new Proxy({}, {get(target, key) { seen.push(key); return undefined; }});
      withPc(value, () => equal(seen,['bundlePolicy','certificates','iceCandidatePoolSize','iceServers','iceTransportPolicy','rtcpMuxPolicy']));
    });
    check('nested dictionary lexical order and iterator single read', () => {
      const seen = [];
      const iterable = {get [Symbol.iterator]() {seen.push('iterator'); return function*() {seen.push('next'); yield {toString(){seen.push('string');return 'stun:host.example';}};seen.push('done');};}};
      const server = {get credential(){seen.push('credential');return undefined;},get urls(){seen.push('urls');return iterable;},get username(){seen.push('username');return undefined;}};
      withPc({iceServers:[server]}, pc => {equal(pc.getConfiguration().iceServers[0].urls,['stun:host.example']); equal(seen,['credential','urls','iterator','next','string','done','username']);});
    });
    check('noniterable URLs fall back to USVString', () => withPc({iceServers:[{urls:{toString(){return 'stun:host.example';},[Symbol.iterator]:null}}]}, pc => equal(pc.getConfiguration().iceServers[0].urls,['stun:host.example'])));
    check('bad iterator propagates before toString', () => {
      let converted=0;
      throws('TypeError', () => new C({iceServers:[{urls:{[Symbol.iterator]:1,toString(){converted++;return 'stun:host.example';}}}]}));
      equal(converted,0);
    });
    check('DOMStrings preserve lone surrogates', () => withPc({iceServers:[{urls:'stun:host.example', username:'\ud800',credential:'\udfff'}]}, pc => {
      equal(pc.getConfiguration().iceServers[0].username,'\ud800');equal(pc.getConfiguration().iceServers[0].credential,'\udfff');
    }));
    for (const value of [-1,256,NaN,Infinity,-Infinity,Symbol('pool')]) {
      check('range pool ' + String(value), () => throws('TypeError', () => new C({iceCandidatePoolSize:value})));
    }
    for (const value of [0,255,255.9,-0.9,'2',null]) {
      check('converted pool ' + String(value), () => withPc({iceCandidatePoolSize:value}, pc => equal(pc.getConfiguration().iceCandidatePoolSize, Math.trunc(Number(value)) || 0)));
    }
    for (const field of ['bundlePolicy','iceTransportPolicy','rtcpMuxPolicy']) {
      check('invalid enum ' + field, () => throws('TypeError', () => new C({[field]:null})));
      check('undefined enum default ' + field, () => withPc({[field]:undefined}, pc => equal(pc.getConfiguration()[field],defaults[field])));
    }
    for (const [index,value] of [null,[null],[undefined],[{}]].entries()) {
      check('invalid servers ' + index, () => throws('TypeError', () => new C({iceServers:value})));
    }
    for (const [index,value] of [null,[null],[{}],[new Proxy({}, {})]].entries()) {
      check('invalid certificates ' + index, () => throws('TypeError', () => new C({certificates:value})));
    }
    const invalid = ['', 'relative', 'https://host.example', 'stun:', 'stun::8191', 'stun:host.example:65536','stun:0:1:2:3:4:5:6:7', 'stun://host.example', 'stun:host.example/path', 'stun:host.example\\path', 'stun:user@host.example', 'stun:host.example#fragment', 'stun:host.example?', 'stun:host.example?transport=udp', 'turn:host.example?invalid', 'turn:host.example?transport=', 'turn:host.example?transport=UDP', 'turn:host.example?transport=datachannel', 'turn:host.example?transport=tcp&extra=1', 'turn:host.example?transport=udp?transport=tcp', 'turn:host.example#', 'turn:user@host.example', 'turn:host.example/path'];
    for (const url of invalid) check('invalid URL ' + JSON.stringify(url), () => throws('SyntaxError', () => new C({iceServers:[{urls:url}]})));
    check('empty URLs invalid', () => throws('SyntaxError', () => new C({iceServers:[{urls:[]}]})));
    for (const url of ['stun:host.example','stuns:host.example:5349','stun:192.0.2.1:3478','stun:[2001:db8::1]:3478','turn:host.example?transport=udp','turns:host.example?transport=tcp']) {
      check('valid URL ' + url, () => withPc({iceServers:[{urls:url,username:'',credential:'secret'}]}, pc => equal(pc.getConfiguration().iceServers[0].urls,[url])));
    }
    for (const credentials of [{},{username:''},{credential:'secret'},{username:'user',credential:''},{username:'é'.repeat(255),credential:'secret'}]) {
      check('TURN credentials ' + JSON.stringify(credentials), () => throws('InvalidAccessError', () => new C({iceServers:[{urls:'turn:host.example',...credentials}]})));
    }
    check('TURN username 509 UTF8 bytes', () => withPc({iceServers:[{urls:'turn:host.example',username:'é'.repeat(254)+'a',credential:'secret'}]}, () => {}));
    check('stored records ignore prototype pollution', () => withPc({iceServers:[{urls:'stun:host.example'}]}, pc => {
      const root=realm.Object.prototype, array=realm.Array.prototype;
      const saved=Object.getOwnPropertyDescriptor(array,Symbol.iterator);
      Object.defineProperty(root,'username',{get(){throw Error('stored record consulted prototype');},configurable:true});
      Object.defineProperty(array,Symbol.iterator,{value(){throw Error('stored arrays consulted iterator');},configurable:true});
      try {const c=pc.getConfiguration();assert(c.iceServers.length===1 && c.iceServers[0].urls[0]==='stun:host.example' && !Object.hasOwn(c.iceServers[0],'username'));}
      finally {delete root.username;Object.defineProperty(array,Symbol.iterator,saved);}
    }));
    check('getter exception identity', () => {
      const sentinel = {};
      let caught;try {new C({get iceServers(){throw sentinel;}});} catch(error){caught=error;}
      assert(caught===sentinel);
    });
    check('setter replacement, snapshot and clear', () => withPc({}, pc => {
      const input = {iceServers:[{urls:'stun:host.example'}],iceTransportPolicy:'relay',iceCandidatePoolSize:2};
      assert(pc.setConfiguration(input)===undefined);input.iceServers[0].urls='stun:changed.example';
      equal(pc.getConfiguration().iceServers,[{urls:['stun:host.example']}]);
      assert(pc.setConfiguration()===undefined);const config=pc.getConfiguration();for(const key of Object.keys(defaults))equal(config[key],defaults[key]);
    }));
    check('failed validation is atomic', () => withPc({iceTransportPolicy:'relay'}, pc => {
      const before=pc.getConfiguration();throws('SyntaxError', () => pc.setConfiguration({iceTransportPolicy:'all',iceServers:[{urls:''}]}));equal(pc.getConfiguration(),before);
    }));
    check('immutable bundle before URL validation', () => withPc({bundlePolicy:'max-bundle'}, pc => {
      throws('InvalidModificationError', () => pc.setConfiguration({bundlePolicy:'max-compat',iceServers:[{urls:''}]}));
      equal(pc.getConfiguration().bundlePolicy,'max-bundle');
      throws('InvalidModificationError', () => pc.setConfiguration());
    }));
    check('closed connection after conversion', () => withPc({}, pc => {
      pc.close();const sentinel={};let caught;
      try {pc.setConfiguration({get iceServers(){throw sentinel;}});}catch(error){caught=error;}assert(caught===sentinel);
      throws('InvalidStateError',()=>pc.setConfiguration({iceServers:[{urls:''}]}));
      equal(pc.getConfiguration().iceTransportPolicy,'all');
    }));
    check('getter reentrancy closes before native mutation', () => withPc({}, pc => {
      throws('InvalidStateError',()=>pc.setConfiguration({get iceServers(){pc.close();return [];}}));
      equal(pc.getConfiguration().iceServers,[]);
    }));
    check('callee snapshot realm', () => withPc({}, pc => {
      const config=pc.getConfiguration();assert(Object.getPrototypeOf(config)===realm.Object.prototype && config.iceServers instanceof realm.Array && config.certificates instanceof realm.Array);
    }));
    check('cross-realm genuine receiver', () => withPc({}, pc => {
      const other = realm===globalThis?document.querySelector('iframe').contentWindow:globalThis;
      other.RTCPeerConnection.prototype.setConfiguration.call(pc,{iceTransportPolicy:'relay'});
      const c=other.RTCPeerConnection.prototype.getConfiguration.call(pc);equal(c.iceTransportPolicy,'relay');assert(Object.getPrototypeOf(c)===other.Object.prototype);
    }));
    withPc({}, real => {
      const revoked=Proxy.revocable(real,{});revoked.revoke();
      let traps=0;
      for (const [name,receiver] of [['empty',{}],['prototype',Object.create(C.prototype)],['inherited',Object.create(real)],['author proxy',new Proxy(real,{get(){traps++;return undefined;}})],['revoked',revoked.proxy]]) {
        check('invalid receiver ' + name, () => {
          throws('TypeError',()=>C.prototype.getConfiguration.call(receiver));
          let converted=0;throws('TypeError',()=>C.prototype.setConfiguration.call(receiver,{get iceServers(){converted++;return [];}}));equal(converted,0);equal(traps,0);
        });
      }
    });
  }
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(c=>c.passed).length,checks};
  return checks.every(c=>c.passed);
})()
