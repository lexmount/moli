use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cache_query_dictionaries_preserve_inheritance_order_and_exceptions() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://cache-options.test/", &loader);
    vm.exec(r#"
      (async () => {
        const cache = await caches.open('dictionary');
        const url = location.href;
        let count = 0;
        for (const [name, call, members] of [
          ['storage.match', options => caches.match(url, options), ['ignoreMethod','ignoreSearch','ignoreVary','cacheName']],
          ...['match','matchAll','keys','delete'].map(name => [name, options => cache[name](url, options), ['ignoreMethod','ignoreSearch','ignoreVary']]),
        ]) {
          let order = [];
          const options = {};
          for (const member of ['cacheName','ignoreVary','ignoreSearch','ignoreMethod'])
            Object.defineProperty(options, member, {get() { order.push(member); return member === 'cacheName' ? 'dictionary' : false; }});
          await call(options);
          if (JSON.stringify(order) !== JSON.stringify(members)) throw Error(name + ': ' + order);
          ++count;
          for (const failing of members) {
            order = [];
            const reason = {failing};
            const abrupt = new Proxy({}, {get(_, member) {
              order.push(member);
              if (member === failing) throw reason;
              return member === 'cacheName' ? 'dictionary' : false;
            }});
            let caught;
            const promise = call(abrupt);
            if (!(promise instanceof Promise)) throw Error('promise result');
            try { await promise; } catch (error) { caught = error; }
            if (caught !== reason || JSON.stringify(order) !== JSON.stringify(members.slice(0, members.indexOf(failing) + 1)))
              throw Error(name + '/abrupt/' + failing);
            ++count;
          }
          for (const primitive of [false, 1, 'options', Symbol('options'), 1n]) {
            const promise = call(primitive);
            let caught;
            try { await promise; } catch (error) { caught = error; }
            if (!(caught instanceof TypeError)) throw Error(name + '/primitive');
            ++count;
          }
          for (const empty of [null, undefined]) { await call(empty); ++count; }
        }
        await caches.delete('dictionary');
        globalThis.cacheDictionaryCount = count;
      })().catch(error => { globalThis.cacheDictionaryError = String(error); });
    "#, None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm, &loader,
        "String(globalThis.cacheDictionaryCount !== undefined || globalThis.cacheDictionaryError !== undefined)",
        "true", "cache-query-dictionaries",
    ).await;
    assert_eq!(
        vm.eval("String(globalThis.cacheDictionaryError)").unwrap(),
        "undefined"
    );
    assert_eq!(vm.eval("cacheDictionaryCount").unwrap(), "56");
}
