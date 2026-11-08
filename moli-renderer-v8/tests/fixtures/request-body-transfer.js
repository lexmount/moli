(async () => {
  globalThis.__blobReadingProgress = {stage: 'setup', checks: [], rows: []};
  async function exercise(source, target, url, prefix) {
    const checks = [], rows = [];
    const check = (name, passed, detail = '') => checks.push({name: `${prefix}/${name}`, passed: !!passed, detail});
    const fence = async () => {for (let i = 0; i < 16; i++) await Promise.resolve();};
    const decode = value => new TextDecoder().decode(value);
    const create = (kind, options = {}) => {
      let body, text = 'data';
      if (kind === 'Text') {body = '\uD800data';text = '\uFFFDdata';}
      else if (kind === 'EmptyText') {body = '';text = '';}
      else if (kind === 'ArrayBuffer') body = new source.Uint8Array([100,97,116,97]).buffer;
      else if (kind === 'Uint8Array') body = new source.Uint8Array([0,100,97,116,97,0]).subarray(1,5);
      else if (kind === 'DataView') body = new source.DataView(new source.Uint8Array([0,100,97,116,97,0]).buffer,1,4);
      else if (kind === 'URLSearchParams') {body = new source.URLSearchParams({a:'before'});text = 'a=before';}
      else if (kind === 'FormData') {body = new source.FormData();body.set('a','before');text = null;}
      else if (kind === 'Blob') body = new source.Blob(['data'], {type:'text/plain'});
      else if (kind === 'File') body = new source.File(['data'],'body.txt',{type:'text/plain'});
      else body = new source.ReadableStream({start(c) {c.enqueue(new source.Uint8Array([100,97,116,97]));c.close();}});
      const input = new source.Request(url, {method:'POST',body,...(kind === 'ReadableStream' ? {duplex:'half'} : {}),...options});
      if (kind === 'URLSearchParams' || kind === 'FormData') body.set('a','after');
      else if (kind === 'Uint8Array') body.fill(0);
      else if (kind === 'ArrayBuffer') new source.Uint8Array(body).fill(0);
      else if (kind === 'DataView') new source.Uint8Array(body.buffer).fill(0);
      const valid = value => text === null ? /name="a"\r\n\r\nbefore\r\n/.test(value) && !value.includes('after') : value === text;
      return {input, valid};
    };
    const kinds = ['Text','EmptyText','ArrayBuffer','Uint8Array','DataView','URLSearchParams','FormData','Blob','File','ReadableStream'];
    for (const kind of kinds) {
      for (const method of ['text','arrayBuffer','bytes','blob']) {
        const label = `${kind}/${method}`;
        const {input,valid} = create(kind), original = input.body, mime = input.headers.get('content-type');
        const transferred = new target.Request(input), stream = transferred.body;
        check(`${label}/input stream locked`, original.locked);
        check(`${label}/input disturbed`, input.bodyUsed);
        check(`${label}/new body unused`, !transferred.bodyUsed && !stream.locked);
        check(`${label}/new stream identity and realm`, stream !== original && Object.getPrototypeOf(stream) === target.ReadableStream.prototype);
        check(`${label}/native MIME retained`, transferred.headers.get('content-type') === mime);
        let byobError = false;
        try {const reader = stream.getReader({mode:'byob'});reader.releaseLock();}
        catch (error) {byobError = error instanceof target.TypeError;}
        check(`${label}/proxy has default reader semantics`, byobError);
        let originalError = false;
        try {await input.text();} catch (error) {originalError = error instanceof source.TypeError;}
        check(`${label}/input cannot be consumed again`, originalError);
        const pending = transferred[method]();
        check(`${label}/promise in method realm`, Object.getPrototypeOf(pending) === target.Promise.prototype);
        let settled = false;
        pending.then(() => {settled = true;},() => {settled = true;});
        await fence();
        check(`${label}/public completion uses a native task`, !settled);
        const result = await pending;
        const value = method === 'text' ? result : method === 'blob' ? await result.text() : decode(result);
        check(`${label}/consumes original immutable data`, valid(value));
        check(`${label}/body used and locked after consumption`, transferred.bodyUsed && stream.locked);
        rows.push({label,inputLocked:original.locked,inputUsed:input.bodyUsed,defaultReaderProxy:byobError});
      }
      {
        const {input,valid} = create(kind);
        const transferred = new target.Request(input), original = transferred.body;
        const clone = transferred.clone();
        check(`${kind}/clone/locks transferred stream`, original.locked);
        check(`${kind}/clone/replaces branch identities`, transferred.body !== original && clone.body !== transferred.body);
        check(`${kind}/clone/input remains disturbed`, input.bodyUsed);
        const values = await Promise.all([transferred.text(),clone.text()]);
        check(`${kind}/clone/both preserve data`, values.every(valid));
        check(`${kind}/clone/both consumed`, transferred.bodyUsed && clone.bodyUsed);
      }
      {
        const {input,valid} = create(kind);
        const middle = new target.Request(input), last = new source.Request(middle);
        check(`${kind}/chain/first disturbed`, input.bodyUsed);
        check(`${kind}/chain/middle disturbed`, middle.bodyUsed && middle.body.locked);
        check(`${kind}/chain/preserves data`, valid(await last.text()));
        check(`${kind}/chain/last consumed`, last.bodyUsed);
      }
      {
        const {input,valid} = create(kind);let error;
        try {new target.Request(input,{method:'GET'});} catch (value) {error = value;}
        check(`${kind}/failure/GET throws`, error instanceof target.TypeError);
        check(`${kind}/failure/GET preserves body state`, !input.bodyUsed && !input.body.locked);
        const sentinel = {};
        try {new target.Request(input,{get headers() {throw sentinel;}});} catch (value) {error = value;}
        check(`${kind}/failure/conversion exception identity`, error === sentinel);
        check(`${kind}/failure/conversion preserves body state`, !input.bodyUsed && !input.body.locked);
        const replacement = new target.Request(input,{body:'replacement'});
        check(`${kind}/override/does not disturb input`, !input.bodyUsed && !input.body.locked);
        check(`${kind}/override/uses new body`, await replacement.text() === 'replacement');
        let originalValue, originalFailure;
        try {originalValue = await input.text();} catch (error) {originalFailure = String(error);}
        check(`${kind}/override/original remains usable`, originalFailure === undefined && valid(originalValue), originalFailure || '');
      }
      {
        const {input,valid} = create(kind);
        const transferred = new target.Request(input,{body:null});
        check(`${kind}/null/inherits body state`, input.bodyUsed && input.body.locked);
        check(`${kind}/null/inherits data`, valid(await transferred.text()));
      }
      if (kind !== 'ReadableStream') {
        const {input,valid} = create(kind,{mode:'no-cors',keepalive:true});
        const transferred = new target.Request(input);
        check(`${kind}/permissions/keeps no-cors`, transferred.mode === 'no-cors');
        check(`${kind}/permissions/keeps keepalive`, transferred.keepalive);
        check(`${kind}/permissions/transfers stream`, input.bodyUsed && input.body.locked);
        check(`${kind}/permissions/retains original data`, valid(await transferred.text()));
      }
    }
    {
      const reason = {cancel:'identity'}, seen = [];
      const stream = new source.ReadableStream({cancel(value) {seen.push(value);}});
      const input = new source.Request(url,{method:'POST',body:stream,duplex:'half'});
      const transferred = new target.Request(input), reader = transferred.body.getReader();
      const pending = reader.read();
      await reader.cancel(reason);
      check('cancel/pending read closes',(await pending).done);
      check('cancel/reaches original source once',seen.length === 1);
      check('cancel/preserves reason identity',seen[0] === reason);
      check('cancel/both bodies used',input.bodyUsed && transferred.bodyUsed);
      reader.releaseLock();
    }
    {
      const reason = {error:'identity'};
      const stream = new source.ReadableStream({start(c) {c.error(reason);}});
      const input = new source.Request(url,{method:'POST',body:stream,duplex:'half'});
      const transferred = new target.Request(input);let error;
      try {await transferred.bytes();} catch (value) {error = value;}
      check('error/preserves source error',error === reason);
      check('error/both bodies used',input.bodyUsed && transferred.bodyUsed);
    }
    return {checks,rows};
  }
  const frame = document.createElement('iframe');document.body.appendChild(frame);
  const realms = [['top',window],['child',frame.contentWindow]];
  for (const [a,source] of realms) for (const [b,target] of realms) {
    __blobReadingProgress.stage = `${a}/${b}`;
    const result = await exercise(source,target,location.href,`${a}/${b}`);
    __blobReadingProgress.checks.push(...result.checks);__blobReadingProgress.rows.push(...result.rows);
  }
  frame.remove();
  __blobReadingProgress.stage = 'worker';
  const code = `(${exercise.toString()})(self,self,${JSON.stringify(location.href)},'worker').then(result=>{postMessage(result);close();},error=>{postMessage({checks:[{name:'worker completion',passed:false,detail:String(error.stack||error)}],rows:[]});close();});`;
  const url = URL.createObjectURL(new Blob([code],{type:'text/javascript'})),worker = new Worker(url);
  try {
    const result = await new Promise((resolve,reject)=>{worker.onmessage=e=>resolve(e.data);worker.onerror=e=>reject(Error(e.message));});
    __blobReadingProgress.checks.push(...result.checks);__blobReadingProgress.rows.push(...result.rows);
  } finally {worker.terminate();URL.revokeObjectURL(url);}
  globalThis.__uiEventResults = {checks:__blobReadingProgress.checks,rows:__blobReadingProgress.rows};
})().catch(error => {
  __blobReadingProgress.checks.push({name:'fixture completion',passed:false,detail:String(error.stack||error)});
  globalThis.__uiEventResults = {checks:__blobReadingProgress.checks,rows:__blobReadingProgress.rows};
});
