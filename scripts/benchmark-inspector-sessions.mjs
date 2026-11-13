#!/usr/bin/env node
// Node.js 22+; two identically configured Release binaries, no npm dependencies.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import net from 'node:net';
import path from 'node:path';
import os from 'node:os';
import { performance } from 'node:perf_hooks';
import { parseArgs } from 'node:util';
const { values: args } = parseArgs({ options: {
  before: {type:'string'}, after: {type:'string'},
  rounds: {type:'string',default:'5'}, output:{type:'string',default:'target/inspector-session-benchmark'},
} });
assert(args.before && args.after, 'Supply --before and --after Release binaries');
assert(Number.isSafeInteger(Number(args.rounds)) && Number(args.rounds)>0, '--rounds must be a positive integer');
await mkdir(args.output,{recursive:true});
class Client {
  constructor(socket) {
    this.socket = socket;
    this.sequence = 1;
    this.pending = new Map();
    this.waiters = [];
    socket.addEventListener('message', ({ data }) => {
      const raw = String(data);
      const message = JSON.parse(raw);
      if (message.id !== undefined) {
        const waiter = this.pending.get(message.id);
        if (!waiter) return;
        this.pending.delete(message.id);
        clearTimeout(waiter.timer);
        if (message.error) waiter.reject(new Error(`${waiter.method}: ${JSON.stringify(message)}`));
        else waiter.resolve({ message, raw });
      } else {
        for (const waiter of [...this.waiters]) {
          if (waiter.matches(message)) {
            this.waiters.splice(this.waiters.indexOf(waiter), 1);
            clearTimeout(waiter.timer);
            waiter.resolve({ message, raw });
          }
        }
      }
    });
    socket.addEventListener('close', () => {
      for (const waiter of [...this.pending.values(), ...this.waiters]) {
        clearTimeout(waiter.timer);
        waiter.reject(new Error('CDP closed with pending work'));
      }
      this.pending.clear();
      this.waiters = [];
    });
  }
  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', reject, { once: true });
    });
    return new Client(socket);
  }
  call(method, params = {}, sessionId) {
    const id = this.sequence++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 30_000);
      this.pending.set(id, { resolve, reject, timer, method });
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }
  event(method, sessionId, predicate = () => true) {
    return new Promise((resolve, reject) => {
      const waiter = { resolve, reject, matches: message => message.method === method && message.sessionId === sessionId && predicate(message) };
      waiter.timer = setTimeout(() => {
        this.waiters.splice(this.waiters.indexOf(waiter), 1);
        reject(new Error(`CDP event timeout: ${method}`));
      }, 30_000);
      this.waiters.push(waiter);
    });
  }
  evaluate(expression, sessionId, returnByValue = true) {
    return this.call('Runtime.evaluate', { expression, returnByValue, awaitPromise: true }, sessionId);
  }
  async nested(method, params, sessionId) {
    const id = this.sequence++;
    const reply = this.event('Target.receivedMessageFromTarget', undefined, message =>
      message.params.sessionId === sessionId && JSON.parse(message.params.message).id === id);
    await this.call('Target.sendMessageToTarget', { sessionId, message: JSON.stringify({ id, method, params }) });
    return JSON.parse((await reply).message.params.message);
  }
  close() { this.socket.close(); }
}

async function start(binary, name) {
  const listener = net.createServer();
  listener.listen(0, '127.0.0.1');
  await once(listener, 'listening');
  const port = listener.address().port;
  await new Promise(resolve => listener.close(resolve));
  const log = await open(path.join(args.output, `${name}.log`), 'w');
  const child = spawn(path.resolve(binary), ['serve', '--host', '127.0.0.1', '--port', String(port), '--log-level', 'error', '--http-no-proxy', '*'], { stdio: ['ignore', 'ignore', log.fd] });
  const exited = once(child, 'exit');
  const deadline = performance.now() + 20_000;
  let version;
  try {
    while (!version) {
      if (child.exitCode !== null) throw new Error(`${name} exited during startup: ${child.exitCode}`);
      try { const response = await fetch(`http://127.0.0.1:${port}/json/version`); if (response.ok) version = await response.json(); } catch { /* Wait for the listening endpoint. */ }
      if (performance.now() > deadline) throw new Error(`${name} did not become ready`);
      if (!version) await new Promise(resolve => setTimeout(resolve, 10));
    }
    const client = await Client.connect(version.webSocketDebuggerUrl);
    const target = (await client.call('Target.createTarget', { url: 'about:blank' })).message.result.targetId;
    const session = (await client.call('Target.attachToTarget', { targetId: target, flatten: true })).message.result.sessionId;
    await client.call('Page.enable', {}, session);
    const loaded = client.event('Page.loadEventFired', session);
    await client.call('Page.navigate', { url: 'data:text/html,<p>Inspector JSON benchmark</p>' }, session);
    await loaded;
    await client.call('Runtime.enable', {}, session);
    return { name, child, client, session, async stop() {
      client.close();
      child.kill('SIGTERM');
      const watchdog = setTimeout(() => child.kill('SIGKILL'), 5_000);
      await exited;
      clearTimeout(watchdog);
      await log.close();
    } };
  } catch (error) {
    child.kill('SIGKILL');
    await exited;
    await log.close();
    throw error;
  }
}


async function usage(pid) {
  const stat = await readFile(`/proc/${pid}/stat`, 'utf8');
  const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
  const status = await readFile(`/proc/${pid}/status`, 'utf8');
  const memory = await readFile(`/proc/${pid}/smaps_rollup`, 'utf8');
  const kib = name => Number(memory.match(new RegExp(`^${name}:\\s+(\\d+)`, 'm'))[1]);
  return {pssKiB:kib("Pss"), privateKiB:kib("Private_Clean")+kib("Private_Dirty"), cpuTicks: Number(fields[11]) + Number(fields[12]),
    rssKiB: Number(status.match(/^VmRSS:\s+(\d+)/m)[1]),
    peakRssKiB: Number(status.match(/^VmHWM:\s+(\d+)/m)[1])};
}
async function measure(server, target, name, iterations, width, operation, verify) {
  for (let i=0;i<10;i++) verify(await operation());
  const before = await usage(server.child.pid);
  const times = [];
  let bytes=0;
  const start = performance.now();
  for(let i=0;i<iterations;i+=width) {
    const count=Math.min(width,iterations-i);
    await Promise.all(Array.from({length:count},async()=>{
      const begin=performance.now(); const response=await operation();
      times.push(performance.now()-begin); bytes+=Buffer.byteLength(response.raw);
      verify(response);
    }));
  }
  const elapsedMs=performance.now()-start, after=await usage(server.child.pid);
  times.sort((a,b)=>a-b);
  return {target,name,iterations,inFlight:width,elapsedMs,bytes,
    medianMs:times[Math.floor(times.length*.5)],p95Ms:times[Math.floor(times.length*.95)],p99Ms:times[Math.min(times.length-1,Math.floor(times.length*.99))],requestsPerSecond:iterations/elapsedMs*1000,
    cpuTicks:after.cpuTicks-before.cpuTicks,rssKiB:after.rssKiB,peakRssKiB:after.peakRssKiB,pssKiB:after.pssKiB,privateKiB:after.privateKiB};
}
async function run(binary, name) {
  const server=await start(binary,name), results=[];
  const {client,session}=server;
  try {
    await client.call('Target.setAutoAttach',{autoAttach:true,waitForDebuggerOnStart:false,flatten:true},session);
    const attached=client.event('Target.attachedToTarget',session,m=>m.params.targetInfo.type==='worker');
    await client.evaluate(`globalThis.benchWorker=new Worker(URL.createObjectURL(new Blob(['onmessage=()=>postMessage(1)'],{type:'text/javascript'})));1`,session);
    const workerAttachment=(await attached).message.params;
    const worker=workerAttachment.sessionId, workerTarget=workerAttachment.targetInfo.targetId;
    await client.call('Runtime.enable',{},worker);
    for(const [target,id] of [['page',session],['worker',worker]]) {
      const group=`bench-${target}`;
      for(const width of [1,8]) {
        for(const [name,expression,iterations,check,options] of [
          ['small',`({owner:'${target}',answer:42})`,300,r=>assert.deepEqual(r.message.result.result.value,{owner:target,answer:42}),{}],
          ['string-1m',`'x'.repeat(1048576)`,30,r=>assert.equal(r.message.result.result.value.length,1048576),{}],
          ['escaped-utf16', `String.fromCharCode(0xd800).repeat(65536) + '\\u0000'`,30,r=>{assert.equal(r.message.result.result.value.length,65537);assert.equal(r.message.result.result.value.charCodeAt(0),0xd800)},{}],
          ['array-by-value',`Array.from({length:10000},(_,i)=>({objectId:'user-'+i,value:i}))`,30,r=>{assert.equal(r.message.result.result.value.length,10000);assert.equal(r.message.result.result.value[9999].value,9999)},{}],
          ['deep-80',`(()=>{let v='x'.repeat(262144);for(let i=0;i<80;i++)v=[v];return v})()`,20,r=>{let v=r.message.result.result.deepSerializedValue;for(let i=0;i<80;i++){assert.equal(v.type,'array');v=v.value[0]}assert.equal(v.value.length,262144)},{serializationOptions:{serialization:'deep',maxDepth:100}}],
        ]) {
          results.push(await measure(server,target,name,iterations,width,
            ()=>client.call('Runtime.evaluate',{expression,returnByValue:true,...options},id),check));
        }
        let consoleSequence=0;
        results.push(await measure(server,target,'console-object-notifications',80,width,async()=>{
          const marker=`console-${target}-${width}-${consoleSequence++}`;
          const event=client.event('Runtime.consoleAPICalled',id,m=>m.params.args[1]?.value===marker);
          await client.evaluate(`console.log({owner:${JSON.stringify(target)}},${JSON.stringify(marker)});42`,id);
          return await event;
        },r=>{assert.equal(r.message.params.args[0].type,'object');if(target==='page')assert(r.message.params.args[0].objectId)}));
        await client.call('Runtime.discardConsoleEntries',{},id);
        results.push(await measure(server,target,'exception-object-results',80,width,
          ()=>client.call('Runtime.evaluate',{expression:"throw new Error('benchmark-exception')",objectGroup:group},id),
          r=>assert(r.message.result.exceptionDetails.exception.objectId)));
        await client.call('Runtime.releaseObjectGroup',{objectGroup:group},id);
        const root=(await client.call('Runtime.evaluate',{expression:`Object.fromEntries(Array.from({length:300},(_,i)=>['p'+i,{owner:'${target}',i}]))`,objectGroup:group},id)).message.result.result.objectId;
        results.push(await measure(server,target,'object-properties',100,width,
          ()=>client.call('Runtime.getProperties',{objectId:root,ownProperties:true},id),
          r=>{assert.equal(r.message.result.result.length,300);assert(r.message.result.result[0].value.objectId)}));
        await client.call('Runtime.releaseObjectGroup',{objectGroup:group},id);
        await assert.rejects(client.call('Runtime.getProperties',{objectId:root},id));
      }
      const before=await usage(server.child.pid),begin=performance.now();
      for(let i=0;i<120;i++) {
        const group=`churn-${target}-${i}`;
        const root=(await client.call('Runtime.evaluate',{expression:`({owner:'${target}',child:{i:${i}}})`,objectGroup:group},id)).message.result.result.objectId;
        const props=(await client.call('Runtime.getProperties',{objectId:root,ownProperties:true},id)).message.result.result;
        assert(props.some(p=>p.name==='child'&&p.value.objectId));
        await client.call('Runtime.releaseObjectGroup',{objectGroup:group},id);
      }
      const after=await usage(server.child.pid);
      results.push({target,name:'group-churn',iterations:120,elapsedMs:performance.now()-begin,cpuTicks:after.cpuTicks-before.cpuTicks,rssKiB:after.rssKiB,peakRssKiB:after.peakRssKiB,pssKiB:after.pssKiB,privateKiB:after.privateKiB});
    }
    const pageTarget=(await client.call('Target.getTargets')).message.result.targetInfos.find(t=>t.type==='page'&&t.url.startsWith('data:text/html')).targetId;
    for (const [target,targetId,survivor] of [['page',pageTarget,session],['worker',workerTarget,worker]]) {
      const before=await usage(server.child.pid),begin=performance.now();
      for(let i=0;i<80;i++) {
        const sid=(await client.call('Target.attachToTarget',{targetId,flatten:true})).message.result.sessionId;
        const value=(await client.call('Runtime.evaluate',{expression:'({root:{value:42}})',...(i%2===0?{objectGroup:'same-name'}:{})},sid)).message.result.result;
        assert(value.objectId);
        const children=(await client.call('Runtime.getProperties',{objectId:value.objectId,ownProperties:true},sid)).message.result.result;
        assert(children[0].value.objectId);
        await client.call('Target.detachFromTarget',{sessionId:sid});
      }
      // A real surviving-session command is the owner progress barrier; no
      // delay is used to guess when native cleanup has completed.
      assert.equal((await client.evaluate('42',survivor)).message.result.result.value,42);
      const after=await usage(server.child.pid);
      results.push({target,name:'attach-detach',iterations:80,elapsedMs:performance.now()-begin,cpuTicks:after.cpuTicks-before.cpuTicks,rssKiB:after.rssKiB,peakRssKiB:after.peakRssKiB,pssKiB:after.pssKiB,privateKiB:after.privateKiB});
    }
    const peer=await Client.connect(client.socket.url);
    try {
      for(const [target,targetId,first] of [['page',pageTarget,session],['worker',workerTarget,worker]]) {
        const second=(await peer.call('Target.attachToTarget',{targetId,flatten:true})).message.result.sessionId;
        const group='same-name-across-clients';
        const roots=await Promise.all([[client,first,'first'],[peer,second,'second']].map(async([connection,sid,owner])=>
          (await connection.call('Runtime.evaluate',{expression:`({owner:${JSON.stringify(owner)},child:{answer:42}})`,objectGroup:group},sid)).message.result.result.objectId));
        let next=0;
        results.push(await measure(server,target,'two-client-properties',160,8,async()=>{
          const index=next++%2;
          const response=await [client,peer][index].call('Runtime.getProperties',{objectId:roots[index],ownProperties:true},[first,second][index]);
          return {...response,expectedOwner:['first','second'][index]};
        },response=>assert.equal(response.message.result.result.find(p=>p.name==='owner').value.value,response.expectedOwner)));
        await client.call('Runtime.releaseObjectGroup',{objectGroup:group},first);
        await assert.rejects(client.call('Runtime.getProperties',{objectId:roots[0]},first));
        const surviving=(await peer.call('Runtime.getProperties',{objectId:roots[1],ownProperties:true},second)).message.result.result;
        assert.equal(surviving.find(p=>p.name==='owner').value.value,'second');
        await peer.call('Target.detachFromTarget',{sessionId:second});
      }
    } finally { peer.close(); }
    const bidiUrl=new URL(client.socket.url);bidiUrl.pathname='/session';
    const bidi=await Client.connect(bidiUrl.toString());
    try {
      await bidi.call('session.new',{});
      const context=(await bidi.call('browsingContext.create',{type:'tab'})).message.result.context;
      await bidi.call('browsingContext.navigate',{context,url:'data:text/html,<p>BiDi benchmark</p>',wait:'complete'});
      const target={context};
      // The baseline BiDi adapter admits one runtime command at a time.
      // CDP concurrency is measured separately above; BiDi remains serial.
      for(const width of [1]) {
        results.push(await measure(server,'bidi-page','by-value-arrays',40,width,
          ()=>bidi.call('script.evaluate',{expression:"Array.from({length:1000},(_,i)=>i)",target,awaitPromise:true,resultOwnership:'none'}),
          r=>{assert.equal(r.message.result.result.type,'array');assert.equal(r.message.result.result.value[999].value,999);assert.equal(r.message.result.result.handle,undefined)}));
        results.push(await measure(server,'bidi-page','root-use-disown',80,width,async()=>{
          const root=await bidi.call('script.evaluate',{expression:'({answer:42})',target,awaitPromise:true,resultOwnership:'root'});
          const handle=root.message.result.result.handle;assert.equal(typeof handle,'string');
          const value=await bidi.call('script.callFunction',{functionDeclaration:'function(){return this.answer}',target,awaitPromise:true,this:{handle}});
          const release=await bidi.call('script.disown',{target,handles:[handle]});
          return {...value,raw:root.raw+value.raw+release.raw};
        },r=>assert.equal(r.message.result.result.value,42)));
      }
      await bidi.call('browsingContext.close',{context});
      await bidi.call('session.end',{});
    } finally { bidi.close(); }
    return {name,results};
  } finally {await server.stop()}
}
const output={node:process.version,machine:{platform:os.platform(),kernel:os.release(),arch:os.arch(),cpu:os.cpus()[0]?.model,cpus:os.availableParallelism()},rounds:Number(args.rounds),binaries:{},observations:[]};
for(const name of ['before','after']) output.binaries[name]={path:path.resolve(args[name]),sha256:createHash('sha256').update(await readFile(args[name])).digest('hex')};
for(let round=0;round<output.rounds;round++) {
  for(const name of round%2===0?['before','after']:['after','before']) {
    const result=await run(args[name],`${name}-${round}`);
    output.observations.push({round,binary:name,results:result.results});
    await writeFile(path.join(args.output,'results.json'),JSON.stringify(output,null,2)+'\n');
    console.log(JSON.stringify({round,binary:name,cases:result.results.length}));
  }
}
