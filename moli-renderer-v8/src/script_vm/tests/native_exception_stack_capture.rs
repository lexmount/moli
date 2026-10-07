use super::*;

#[test]
fn native_exception_stacks_ignore_mutated_javascript_capture_helpers() {
    let mut vm = new_parsed_test_vm(
        "https://dom-exception-native-stacks.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
 const child=document.body.appendChild(document.createElement('iframe')).contentWindow;
 for(const realm of [window,child]) {
  const previous=Object.getOwnPropertyDescriptor(realm.Error,'captureStackTrace');
  Object.defineProperty(realm.Error,'captureStackTrace',{configurable:true,get(){throw Error('author lookup');}});
  try {
   for(const error of [new realm.DOMException('message','AbortError'),new realm.QuotaExceededError('full'),new realm.WebSocketError('closed')]) {
    const descriptor=Object.getOwnPropertyDescriptor(error,'stack');
    if(typeof error.stack!=='string'||!error.stack.startsWith(error.name+': '+error.message)||!descriptor||descriptor.enumerable)throw Error('native stack');
   }
   let error;try{realm.document.querySelector('[');}catch(caught){error=caught;}
   if(!(error instanceof realm.DOMException)||typeof error.stack!=='string')throw Error('factory stack');
  } finally {if(previous)Object.defineProperty(realm.Error,'captureStackTrace',previous);else delete realm.Error.captureStackTrace;}
 }
 return 'ok';
})()"#).expect("native regression"), "ok");
}
