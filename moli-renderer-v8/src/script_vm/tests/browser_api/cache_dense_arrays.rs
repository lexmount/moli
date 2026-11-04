use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cache_results_have_own_entries_despite_inherited_numeric_properties() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://cache-dense-arrays.test/",
        &loader,
    );
    vm.exec(r#"
      (async () => {
        const cache = await caches.open('dense-results');
        const urls = [location.origin + '/first', location.origin + '/second'];
        for (let i = 0; i < urls.length; ++i)
          await cache.put(urls[i], new Response('body-' + i, {headers: {'x-test': 'value'}}));
        let count = 0;
        for (const api of ['keys', 'matchAll']) {
          for (const prototype of [Array.prototype, Object.prototype]) {
            for (const index of ['0', '1']) {
              for (const mode of ['setter', 'throwing-setter', 'getter-only', 'readonly']) {
                const previous = Object.getOwnPropertyDescriptor(prototype, index);
                let calls = 0, result, failure;
                const descriptor = mode === 'readonly'
                  ? {configurable: true, value: 'inherited', writable: false}
                  : mode === 'getter-only'
                    ? {configurable: true, get() { ++calls; throw Error('getter'); }}
                    : {configurable: true, set() { ++calls; if (mode === 'throwing-setter') throw Error('setter'); }};
                Object.defineProperty(prototype, index, descriptor);
                try { result = await cache[api](); } catch (error) { failure = error; }
                finally {
                  if (previous) Object.defineProperty(prototype, index, previous);
                  else delete prototype[index];
                }
                if (failure || calls || !Array.isArray(result) || result.length !== 2)
                  throw Error(api + '/' + index + '/' + mode + ': ' + String(failure) + ' calls=' + calls);
                for (let i = 0; i < result.length; ++i) {
                  const own = Object.getOwnPropertyDescriptor(result, String(i));
                  if (!own || !('value' in own) || !own.enumerable) throw Error('missing own entry');
                  if (api === 'keys') {
                    if (!(own.value instanceof Request) || own.value.url !== urls[i]) throw Error('request');
                  } else if (!(own.value instanceof Response) || await own.value.text() !== 'body-' + i)
                    throw Error('response');
                }
                ++count;
              }
            }
          }
        }
        for (let i = 0; i < urls.length; ++i)
          if (await (await cache.match(urls[i])).text() !== 'body-' + i) throw Error('stored body changed');
        await caches.delete('dense-results');
        globalThis.cacheArrayCount = count;
      })().catch(error => { globalThis.cacheArrayError = String(error); })
    "#, None).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm, &loader,
        "String(globalThis.cacheArrayCount !== undefined || globalThis.cacheArrayError !== undefined)",
        "true", "cache-dense-result-arrays",
    ).await;
    assert_eq!(
        vm.eval("String(globalThis.cacheArrayError)").unwrap(),
        "undefined"
    );
    assert_eq!(vm.eval("globalThis.cacheArrayCount").unwrap(), "32");
}
