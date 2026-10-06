use super::*;

#[test]
fn domrect_native_receiver_checks_preserve_interface_and_callee_realm() {
    let mut vm = new_parsed_test_vm(
        "https://domrect_receiver_consolidation.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
 const child=document.body.appendChild(document.createElement('iframe')).contentWindow;
 let conversions=0,traps=0;
 const value={valueOf(){conversions++;return 7;}};
 for(const realm of [window,child]){
  const mutable=new realm.DOMRect(1,2,3,4),readonly=new realm.DOMRectReadOnly(1,2,3,4);
  const revoked=Proxy.revocable(mutable,{});revoked.revoke();
  const invalid=[{},Object.create(mutable),Object.create(realm.DOMRect.prototype),new Proxy(mutable,{get(){traps++;throw Error('trap');}}),revoked.proxy];
  for(const name of ['x','y','width','height']){
   const getter=Object.getOwnPropertyDescriptor(realm.DOMRectReadOnly.prototype,name).get;
   const setter=Object.getOwnPropertyDescriptor(realm.DOMRect.prototype,name).set;
   for(const target of invalid){
    for(const invoke of [()=>getter.call(target),()=>setter.call(target,value)]){
     let caught;try{invoke();}catch(error){caught=error;}
     if(!(caught instanceof realm.TypeError))throw Error(name+' receiver realm');
    }
   }
   let caught;try{setter.call(readonly,value);}catch(error){caught=error;}
   if(!(caught instanceof realm.TypeError))throw Error('readonly accepted mutable setter');
  }
  const foreign=realm===window?new child.DOMRect(5,6,7,8):new DOMRect(5,6,7,8);
  const setter=Object.getOwnPropertyDescriptor(realm.DOMRect.prototype,'x').set;
  setter.call(foreign,9);if(foreign.x!==9)throw Error('genuine foreign DOMRect');
  for(const target of invalid){let caught;try{realm.DOMRectReadOnly.prototype.toJSON.call(target);}catch(error){caught=error;}if(!(caught instanceof realm.TypeError))throw Error('toJSON receiver');}
 }
 if(conversions||traps)throw Error('receiver validation executed author code');
 return 'ok';
})()"#).expect("native binding regression"), "ok");
}
