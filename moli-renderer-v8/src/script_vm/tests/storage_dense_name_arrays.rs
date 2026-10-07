use super::*;

#[tokio::test(flavor = "current_thread")]
async fn storage_name_arrays_ignore_numeric_prototype_properties() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-dense-name-arrays.test/",
        &loader,
    );
    assert_eq!(vm.eval(r#"(() => {
 const own=Object.getOwnPropertyDescriptor,define=Object.defineProperty;
 globalThis.__storageNameArrayFacts={complete:false,error:null};
 (async()=>{
  const apis=[navigator.storageBuckets,caches];
  for(let a=0;a<apis.length;a++) {
   const api=apis[a],name='dense-names-'+a,second=name+'-second';await api.open(name);await api.open(second);
   const expected=await api.keys();if(expected.length<2)throw Error('fixture needs both indices');
   for(const parent of [Array.prototype,Object.prototype])for(const index of ['0','1'])for(const kind of ['setter','throwing','getter','readonly']) {
    const old=own(parent,index);let calls=0,result;
    const descriptor=kind==='readonly'?{configurable:true,value:99,writable:false}:kind==='getter'?{configurable:true,get(){calls++;throw 42;}}:{configurable:true,set(){calls++;if(kind==='throwing')throw 43;}};
    define(parent,index,descriptor);
    try{result=await api.keys();}finally{if(old)define(parent,index,old);else delete parent[index];}
    if(calls||!Array.isArray(result)||Object.isFrozen(result)||JSON.stringify(result)!==JSON.stringify(expected))throw Error('dense names');
    for(let i=0;i<result.length;i++){const d=own(result,String(i));if(!d||!d.writable||!d.configurable||!d.enumerable||!('value' in d))throw Error('own item');}
   }
   await api.delete(name);await api.delete(second);
  }
 })().catch(error=>{__storageNameArrayFacts.error=String(error);}).finally(()=>{__storageNameArrayFacts.complete=true;});
 return 'ok';
})()"#).expect("native regression"), "ok");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__storageNameArrayFacts.complete)",
        "true",
        "dense-storage-names",
    )
    .await;
    assert_eq!(
        vm.eval("String(__storageNameArrayFacts.error)").unwrap(),
        "null"
    );
}
