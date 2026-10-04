use super::*;

#[test]
fn credential_members_preserve_native_receiver_inheritance_and_error_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://credential-interfaces.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let objects = v8::Object::new(scope);
        let proxies = v8::Object::new(scope);
        for interface in [
            "Credential",
            "PublicKeyCredential",
            "AuthenticatorResponse",
            "AuthenticatorAttestationResponse",
            "AuthenticatorAssertionResponse",
        ] {
            let prototype =
                crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, interface)?;
            let object = v8::Object::new(scope);
            assert_eq!(object.set_prototype(scope, prototype.into()), Some(true));
            // Native identity fixtures only, not produced credentials or authentication results.
            moli_webapi_declare::initialize_web_api_object(scope, object, interface).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, interface);
            assert_eq!(
                objects.create_data_property(scope, key.into(), object.into()),
                Some(true)
            );
            assert_eq!(
                proxies.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        for (name, object) in [
            ("nativeCredentials", objects),
            ("nativeCredentialProxies", proxies),
        ] {
            let key = crate::util::v8str(scope, name);
            assert_eq!(
                global.create_data_property(scope, key.into(), object.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const foreign=document.getElementById('child').contentWindow;
      const members=[
        ['Credential',['id','type'],[]],
        ['PublicKeyCredential',['rawId','response','authenticatorAttachment'],['getClientExtensionResults','toJSON']],
        ['AuthenticatorResponse',['clientDataJSON'],[]],
        ['AuthenticatorAttestationResponse',['attestationObject'],['getTransports','getAuthenticatorData','getPublicKey','getPublicKeyAlgorithm']],
        ['AuthenticatorAssertionResponse',['authenticatorData','signature','userHandle'],[]]
      ];
      for(const [name,attributes,methods] of members) {
        const callbacks=[...attributes.map(key=>Object.getOwnPropertyDescriptor(foreign[name].prototype,key)?.get),...methods.map(key=>foreign[name].prototype[key])];
        const native=nativeCredentials[name],proxy=nativeCredentialProxies[name];
        Object.setPrototypeOf(native,null);
        globalThis[name]=function AuthorConstructor(){throw Error('author constructor');};
        const derived=name==='Credential'?['PublicKeyCredential']:name==='AuthenticatorResponse'?['AuthenticatorAttestationResponse','AuthenticatorAssertionResponse']:[];
        const valid=[native,proxy,...derived.flatMap(key=>[nativeCredentials[key],nativeCredentialProxies[key]])];
        for(const callback of callbacks) {
          if(typeof callback!=='function') throw Error('Missing native member');
          for(const value of valid) {
            let error;try{callback.call(value);}catch(e){error=e;}
            if(!(error instanceof foreign.DOMException)||error.name!=='NotSupportedError'||error.code!==9) throw Error('Native brand or error realm');
          }
          let traps=0;const trap=()=>{traps++;throw Error('author trap');};
          const revoked=Proxy.revocable(native,{});revoked.revoke();
          for(const value of [{},Object.create(native),new Proxy(native,{get:trap,getPrototypeOf:trap}),revoked.proxy,new Event('wrong-interface')]) {
            let error;try{callback.call(value);}catch(e){error=e;}
            if(!(error instanceof foreign.TypeError)) throw Error('Invalid receiver accepted');
          }
          if(traps!==0) throw Error('Brand check executed author trap');
        }
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn credential_availability_queries_use_native_promises_and_callee_records() {
    let mut vm = new_storage_page_task_executor_test_vm("https://credential-capabilities.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(r#"(async () => {
      const assert=(ok,message)=>{if(!ok)throw Error(message);};
      const foreign=document.getElementById('child').contentWindow;
      const PromiseCtor=foreign.Promise,ObjectPrototype=foreign.Object.prototype;
      const conditional=foreign.PublicKeyCredential.isConditionalMediationAvailable;
      const platform=foreign.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable;
      const capabilities=foreign.PublicKeyCredential.getClientCapabilities;
      const baseConditional=foreign.Credential.isConditionalMediationAvailable;
      const keys=['conditionalCreate','conditionalGet','hybridTransport','passkeyPlatformAuthenticator','relatedOrigins','signalAllAcceptedCredentials','signalCurrentUserDetails','signalUnknownCredential','userVerifyingPlatformAuthenticator'];
      foreign.PublicKeyCredential=function AuthorConstructor(){throw Error('author constructor');};
      foreign.Credential=function AuthorConstructor(){throw Error('author constructor');};
      foreign.Promise=function AuthorPromise(){throw Error('author Promise');};
      let traps=0;const trap=()=>{traps++;throw Error('author trap');};
      const revoked=Proxy.revocable({},{});revoked.revoke();
      for(const thisArg of [null,{},revoked.proxy,new Proxy({}, {get:trap,getPrototypeOf:trap})]) {
        for(const query of [conditional,platform,baseConditional]) {
          const promise=query.call(thisArg,{toString:trap});
          assert(promise instanceof PromiseCtor,'Promise belongs to callee intrinsic realm');
          let called=false;promise.then(()=>{called=true;});assert(!called,'Promise callbacks are asynchronous');
          assert(await promise===false,'No unsupported authentication capability advertised');
        }
        const promise=capabilities.call(thisArg);
        assert(promise instanceof PromiseCtor,'Capabilities promise belongs to callee realm');
        const first=await promise,second=await capabilities();
        assert(Object.getPrototypeOf(first)===ObjectPrototype,'Record belongs to callee realm');
        assert(JSON.stringify(Object.keys(first))===JSON.stringify(keys),'Sorted capability keys');
        assert(keys.every(key=>first[key]===false),'All unsupported capabilities are false');
        first.conditionalGet=true;
        assert(first!==second&&second.conditionalGet===false,'No shared mutable capability record');
      }
      assert(traps===0,'No author receiver or argument conversion');
      globalThis.__credentialAvailabilityDone=true;
    })().catch(error=>globalThis.__credentialAvailabilityDone=String(error))"#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("__credentialAvailabilityDone")
            .unwrap(),
        "true"
    );
}
