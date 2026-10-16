(async () => {
  globalThis.__blobReadingProgress = {stage:'setup',checks:[],rows:[]};
  async function exercise(source,target,url,prefix) {
    const checks=[],rows=[];
    const check=(name,passed,detail='')=>checks.push({name:`${prefix}/${name}`,passed:!!passed,detail});
    const decode=value=>new TextDecoder().decode(value);
    const create=kind=>{
      let body,text='data';
      if(kind==='Text') body='data';
      else if(kind==='EmptyText') {body='';text='';}
      else if(kind==='ArrayBuffer') body=new source.Uint8Array([100,97,116,97]).buffer;
      else if(kind==='Uint8Array') body=new source.Uint8Array([0,100,97,116,97,0]).subarray(1,5);
      else if(kind==='DataView') body=new source.DataView(new source.Uint8Array([0,100,97,116,97,0]).buffer,1,4);
      else if(kind==='URLSearchParams') {body=new source.URLSearchParams({a:'before'});text='a=before';}
      else if(kind==='FormData') {body=new source.FormData();body.set('a','before');text=null;}
      else if(kind==='Blob') body=new source.Blob(['data'],{type:'text/plain'});
      else if(kind==='File') body=new source.File(['data'],'body.txt',{type:'text/plain'});
      else body=new source.ReadableStream({start(c){c.enqueue(new source.Uint8Array([100,97,116,97]));c.close();}});
      const response=new target.Response(body,{status:201,statusText:'Created',headers:{'x-original':'one'}});
      if(kind==='URLSearchParams'||kind==='FormData')body.set('a','after');
      else if(kind==='Uint8Array')body.fill(0);
      else if(kind==='ArrayBuffer')new source.Uint8Array(body).fill(0);
      else if(kind==='DataView')new source.Uint8Array(body.buffer).fill(0);
      const valid=value=>text===null?/name="a"\r\n\r\nbefore\r\n/.test(value)&&!value.includes('after'):value===text;
      return {response,valid};
    };
    const kinds=['Text','EmptyText','ArrayBuffer','Uint8Array','DataView','URLSearchParams','FormData','Blob','File','ReadableStream'];
    const cloneMethod=source.Response.prototype.clone;
    for(const kind of kinds){
      for(const method of ['text','arrayBuffer','bytes','blob']){
        const {response,valid}=create(kind),original=response.body,mime=response.headers.get('content-type');
        const clone=cloneMethod.call(response);
        check(`${kind}/${method}/locks original stream`,original.locked);
        check(`${kind}/${method}/replaces original body`,response.body!==original);
        check(`${kind}/${method}/independent branches`,response.body!==clone.body);
        check(`${kind}/${method}/both bodies initially unused`,!response.bodyUsed&&!clone.bodyUsed);
        check(`${kind}/${method}/clone and headers receiver realm`,Object.getPrototypeOf(clone)===target.Response.prototype&&clone.headers instanceof target.Headers);
        check(`${kind}/${method}/copies metadata`,clone.status===201&&clone.statusText==='Created'&&clone.type===response.type&&clone.url===response.url&&clone.headers.get('content-type')===mime);
        clone.headers.set('x-original','two');
        check(`${kind}/${method}/header lists independent`,response.headers.get('x-original')==='one');
        const consume=async body=>{
          const result=await body[method]();
          return method==='text'?result:method==='blob'?await result.text():decode(result);
        };
        const values=await Promise.all([consume(response),consume(clone)]);
        check(`${kind}/${method}/both preserve immutable data`,values.every(valid));
        check(`${kind}/${method}/both used and locked`,response.bodyUsed&&clone.bodyUsed&&response.body.locked&&clone.body.locked);
      }
      {
        const {response,valid}=create(kind),original=response.body;
        const first=cloneMethod.call(response),second=cloneMethod.call(first);
        check(`${kind}/tree/original stream locked`,original.locked);
        check(`${kind}/tree/all initially unused`,![response,first,second].some(value=>value.bodyUsed));
        const values=await Promise.all([response.text(),first.text(),second.text()]);
        check(`${kind}/tree/retains all data`,values.every(valid));
      }
      {
        const {response,valid}=create(kind),original=response.body;
        const reader=original.getReader();let error;
        try{cloneMethod.call(response);}catch(value){error=value;}
        check(`${kind}/locked/rejects in method realm`,error instanceof source.TypeError);
        check(`${kind}/locked/retains original body`,response.body===original&&!response.bodyUsed);
        reader.releaseLock();
        check(`${kind}/locked/input remains usable`,valid(await response.text()));
        try{cloneMethod.call(response);}catch(value){error=value;}
        check(`${kind}/used/rejects clone`,error instanceof source.TypeError);
      }
      if(kind!=='ReadableStream'){
        const {response}=create(kind),clone=cloneMethod.call(response);
        const a=response.body.getReader({mode:'byob'}),b=clone.body.getReader({mode:'byob'});
        const one=new target.Uint8Array(256),two=new target.Uint8Array(256);
        const [x,y]=await Promise.all([a.read(one),b.read(two)]);
        check(`${kind}/BYOB/transfers both buffers`,one.buffer.byteLength===0&&two.buffer.byteLength===0);
        check(`${kind}/BYOB/same data and done`,x.done===y.done&&decode(x.value)===decode(y.value));
        if(!x.done){
          const last=await Promise.all([a.read(new target.Uint8Array(1)),b.read(new target.Uint8Array(1))]);
          check(`${kind}/BYOB/both branches close`,last.every(result=>result.done));
        }else check(`${kind}/BYOB/both branches close`,true);
        a.releaseLock();b.releaseLock();
      }
    }
    {
      const {response}=create('Text'),original=response.body,clone=cloneMethod.call(response);
      const a=response.body.getReader(),b=clone.body.getReader();
      const [one,two]=await Promise.all([a.read(),b.read()]);
      const before=decode(two.value);one.value[0]=0;
      check('byte copy/branch buffers independent',one.value.buffer!==two.value.buffer&&decode(two.value)===before);
      check('byte copy/original stream tee locked',original.locked);
      a.releaseLock();b.releaseLock();
    }
    for(const kind of ['none','error','no-content']){
      const response=kind==='error'?target.Response.error():kind==='none'?new target.Response():new target.Response(null,{status:204});
      const clone=cloneMethod.call(response);
      check(`${kind}/null bodies remain null`,response.body===null&&clone.body===null);
      check(`${kind}/metadata preserved`,response.status===clone.status&&response.type===clone.type);
      check(`${kind}/empty consumption`,await clone.text()==='');
      check(`${kind}/both remain unused`,!response.bodyUsed&&!clone.bodyUsed);
    }
    return {checks,rows};
  }
  const frame=document.createElement('iframe');document.body.appendChild(frame);
  const realms=[['top',window],['child',frame.contentWindow]];
  for(const [a,source] of realms)for(const [b,target] of realms){
    __blobReadingProgress.stage=`${a}/${b}`;
    const result=await exercise(source,target,location.href,`${a}/${b}`);
    __blobReadingProgress.checks.push(...result.checks);
  }
  frame.remove();
  const code=`(${exercise.toString()})(self,self,${JSON.stringify(location.href)},'worker').then(result=>{postMessage(result);close();},error=>{postMessage({checks:[{name:'worker completion',passed:false,detail:String(error.stack||error)}]});close();});`;
  const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'})),worker=new Worker(url);
  try{
    const result=await new Promise((resolve,reject)=>{worker.onmessage=e=>resolve(e.data);worker.onerror=e=>reject(Error(e.message));});
    __blobReadingProgress.checks.push(...result.checks);
  }finally{worker.terminate();URL.revokeObjectURL(url);}
  globalThis.__uiEventResults={checks:__blobReadingProgress.checks};
})().catch(error=>{
  __blobReadingProgress.checks.push({name:'fixture completion',passed:false,detail:String(error.stack||error)});
  globalThis.__uiEventResults={checks:__blobReadingProgress.checks};
});
