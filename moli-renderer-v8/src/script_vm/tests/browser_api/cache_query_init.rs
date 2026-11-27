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

#[test]
fn cache_match_all_and_keys_return_frozen_arrays() {
    let mut vm = new_storage_page_task_executor_test_vm("https://cache-results.test/");
    vm.eval(
        r#"
        globalThis.cacheArrayResult = 'pending';
        (async () => {
            const cache = await caches.open('frozen-results');
            const empty = [await cache.matchAll(), await cache.keys()];
            await cache.put('/entry', new Response('body'));
            const responses = await cache.matchAll();
            const requests = await cache.keys();
            return JSON.stringify([
                ...empty.map(Object.isFrozen),
                Object.isFrozen(responses), Object.isFrozen(requests),
                responses[0] instanceof Response, requests[0] instanceof Request,
                responses !== await cache.matchAll(), requests !== await cache.keys(),
            ]);
        })().then(value => cacheArrayResult = value, error => cacheArrayResult = String(error));
        "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("cacheArrayResult")
            .unwrap(),
        "[true,true,true,true,true,true,true,true]"
    );
}
