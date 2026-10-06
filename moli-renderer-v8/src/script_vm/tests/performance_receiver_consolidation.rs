use super::*;

#[test]
fn performance_bindings_validate_native_receivers_before_conversion() {
    let mut vm = new_parsed_test_vm(
        "https://performance_receiver_consolidation.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
 const child=document.body.appendChild(document.createElement('iframe')).contentWindow;
 let conversions=0,traps=0;
 const argument={toString(){conversions++;return 'entry';},valueOf(){conversions++;return 1;}};
 for(const realm of [window,child]) {
  const real=realm.performance,proto=realm.Performance.prototype;
  const revoked=Proxy.revocable(real,{});revoked.revoke();
  const invalid=[{},Object.create(real),Object.create(proto),new Proxy(real,{get(){traps++;throw Error('trap');}}),revoked.proxy];
  const calls=['now','mark','measure','clearMarks','clearMeasures','getEntries','getEntriesByName','getEntriesByType','toJSON','clearResourceTimings','setResourceTimingBufferSize'].map(name=>[name,(target)=>proto[name].call(target,argument)]);
  for(const name of ['timeOrigin','memory','navigation','timing','eventCounts','onresourcetimingbufferfull']) {
   const descriptor=Object.getOwnPropertyDescriptor(proto,name);
   if(!descriptor)throw Error('missing '+name);
   calls.push([name,(target)=>descriptor.get.call(target)]);
   if(descriptor.set)calls.push([name+' setter',(target)=>descriptor.set.call(target,argument)]);
  }
  for(const [name,invoke] of calls)for(const target of invalid){
   let caught;try{invoke(target);}catch(error){caught=error;}
   if(!(caught instanceof realm.TypeError))throw Error(name+' did not reject invalid receiver in callee realm');
  }
  const getter=Object.getOwnPropertyDescriptor(proto,'timeOrigin').get;
  for(const target of [performance,child.performance])if(getter.call(target)!==target.timeOrigin)throw Error('genuine cross-realm receiver');
 }
 if(conversions||traps)throw Error('receiver validation executed author code');
 return 'ok';
})()"#).expect("native binding regression"), "ok");
}
