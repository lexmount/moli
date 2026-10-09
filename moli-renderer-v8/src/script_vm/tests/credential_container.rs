use super::*;

#[test]
fn credentials_container_preserves_native_brands_conversion_order_abort_and_inactive_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://credentials-container.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("credential_container.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn credentials_container_exposes_navigator_property_in_secure_contexts() {
    for (url, exposed) in [
        ("https://credentials-container.test/", true),
        ("http://localhost/", true),
        ("http://credentials-container.test/", false),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm(url);
        assert_eq!(
            vm.eval("Object.hasOwn(Navigator.prototype, 'credentials')")
                .unwrap(),
            exposed.to_string(),
            "{url}"
        );
    }
}

#[test]
fn credentials_container_accepts_registered_native_proxies_and_native_credential_arguments() {
    let mut vm = new_storage_page_task_executor_test_vm("https://credentials-container.test/");
    vm.eval("globalThis.nativeSignal = AbortSignal.abort({})")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let fixtures = v8::Object::new(scope);
        for name in ["CredentialsContainer", "Credential", "PublicKeyCredential"] {
            let object = v8::Object::new(scope);
            // Identity-only fixtures, not produced credentials or saved secrets.
            moli_webapi_declare::initialize_web_api_object(scope, object, name).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, name);
            assert_eq!(
                fixtures.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        let key = crate::util::v8str(scope, "navigator");
        let navigator = global.get(scope, key.into()).unwrap();
        let navigator = v8::Local::<v8::Object>::try_from(navigator).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, navigator, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "Navigator");
        assert_eq!(
            fixtures.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        let key = crate::util::v8str(scope, "nativeSignal");
        let signal = global.get(scope, key.into()).unwrap();
        let signal = v8::Local::<v8::Object>::try_from(signal).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, signal, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "AbortSignal");
        assert_eq!(
            fixtures.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        let key = crate::util::v8str(scope, "nativeFixtures");
        assert_eq!(
            global.create_data_property(scope, key.into(), fixtures.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.eval(r#"(async () => {
      const prototype = CredentialsContainer.prototype, container = nativeFixtures.CredentialsContainer;
      const getter = Object.getOwnPropertyDescriptor(Navigator.prototype, 'credentials').get;
      if (getter.call(nativeFixtures.Navigator) !== navigator.credentials) throw Error('Native Navigator proxy rejected');
      if (await prototype.preventSilentAccess.call(container) !== undefined) throw Error('Native proxy receiver rejected');
      for (const method of ['get', 'create']) {
        let reason; try { await prototype[method].call(container, {signal: nativeFixtures.AbortSignal}); } catch (caught) { reason = caught; }
        if (reason !== nativeSignal.reason) throw Error('Native AbortSignal proxy rejected');
      }
      for (const credential of [nativeFixtures.Credential, nativeFixtures.PublicKeyCredential]) {
        let error; try { await prototype.store.call(container, credential); } catch (caught) { error = caught; }
        if (!(error instanceof DOMException) || error.name !== 'NotSupportedError') throw Error('Native Credential brand rejected');
        let conversions = 0;
        const revoked = Proxy.revocable(credential, {}); revoked.revoke();
        for (const fake of [Object.create(credential), new Proxy(credential, {get() { conversions++; throw Error('Trap'); }}), revoked.proxy]) {
          let error; try { await prototype.store.call(container, fake); } catch (caught) { error = caught; }
          if (!(error instanceof TypeError)) throw Error('Author receiver accepted');
        }
        if (conversions) throw Error('Native interface conversion ran author trap');
      }
      globalThis.nativeCredentialTestDone = true;
    })().catch(error => globalThis.nativeCredentialTestDone = String(error))"#).unwrap();
    assert_eq!(
        vm.eval("globalThis.nativeCredentialTestDone").unwrap(),
        "true"
    );
}
