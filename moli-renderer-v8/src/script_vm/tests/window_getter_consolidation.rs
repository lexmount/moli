use super::*;

#[test]
fn replaceable_window_getters_use_native_receiver_validation() {
    let mut vm = new_parsed_test_vm(
        "https://window_getter_consolidation.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
 const child=document.body.appendChild(document.createElement('iframe')).contentWindow;
 for(const realm of [window,child])for(const name of ['innerWidth','outerHeight','event']){
  const descriptor=Object.getOwnPropertyDescriptor(realm,name);
  for(const target of [window,child])if(descriptor.get.call(target)!==target[name])throw Error('receiver-owned value '+name);
  for(const target of [{},Object.create(realm),new Proxy(realm,{})]){
   let caught;try{descriptor.get.call(target);}catch(error){caught=error;}
   if(!(caught instanceof realm.TypeError))throw Error('invalid receiver '+name);
  }
  const before=descriptor.get.call(realm),replacement={};descriptor.set.call(realm,replacement);
  if(realm[name]!==replacement||descriptor.get.call(realm)!==before)throw Error('replaceable getter state');
  Object.defineProperty(realm,name,descriptor);
 }
 return 'ok';
})()"#).expect("native binding regression"), "ok");
}
