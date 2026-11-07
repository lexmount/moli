use super::*;

#[test]
fn cache_query_converts_inherited_options_before_native_request_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://cache-dictionary.test/app/");
    vm.eval(&format!(
        "globalThis.cacheDictionaryResult='pending'; ({}).then(value=>cacheDictionaryResult=value,error=>cacheDictionaryResult=String(error.stack||error));",
        include_str!("../../../../tests/fixtures/cache-query-dictionary.js"),
    ))
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(cacheDictionaryResult)")
            .unwrap(),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cache_storage_match_with_missing_name_returns_undefined_without_creating_a_cache() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://cache-name-match.test/",
        &loader,
    );
    vm.eval(
        r#"
      globalThis.namedMatchResult = null;
      (async () => {
        const cache = await caches.open('existing');
        await cache.put('/value', new Response('stored'));
        const before = await caches.keys();
        for (const name of ['missing', '']) {
          if (await caches.match('/value', {cacheName: name}) !== undefined)
            throw new Error('missing named cache matched another cache');
          if (await caches.has(name)) throw new Error('match created a cache');
        }
        if (JSON.stringify(await caches.keys()) !== JSON.stringify(before))
          throw new Error('match changed cache names');
        if (await (await caches.match('/value', {cacheName: 'existing'})).text() !== 'stored')
          throw new Error('existing named cache did not match');
        await caches.delete('existing');
        if (await caches.match('/value', {cacheName: 'existing'}) !== undefined)
          throw new Error('deleted named cache matched');
        return 'passed';
      })().then(value => namedMatchResult = value, error => namedMatchResult = String(error));
    "#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(namedMatchResult !== null)",
        "true",
        "CacheStorage named match",
    )
    .await;
    assert_eq!(vm.eval("namedMatchResult").unwrap(), "passed");
}
