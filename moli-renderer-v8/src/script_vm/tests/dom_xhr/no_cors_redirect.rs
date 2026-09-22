use super::*;

fn redirect_filter_probe_script(probe: &str, worker: bool) -> String {
    if worker {
        let source = format!(
            "Promise.resolve().then(() => {probe}).then(value => {{ postMessage(value); close(); }}, error => {{ postMessage(String(error.stack || error)); close(); }});"
        );
        format!(
            "globalThis.redirectFilterResult = 'pending'; const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}}))); worker.onmessage = event => {{ redirectFilterResult = event.data; }}; worker.onerror = event => {{ redirectFilterResult = event.message; event.preventDefault(); }};",
            serde_json::to_string(&source).unwrap()
        )
    } else {
        format!(
            "globalThis.redirectFilterResult = 'pending'; Promise.resolve().then(() => {probe}).then(value => {{ redirectFilterResult = value; }}, error => {{ redirectFilterResult = String(error.stack || error); }});"
        )
    }
}

async fn check_no_cors_redirect_before_interception(worker: bool) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://redirect-filter.test/page", &loader);
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    let probe = r#"(async () => {
      const url = 'https://remote.redirect-filter.test/response';
      const assert = (value, label) => { if (!value) throw new Error(label); };
      const rejected = async (promise, check, label) => {
        let reason, failed = false;
        try { await promise; } catch (error) { reason = error; failed = true; }
        assert(failed && check(reason), label);
      };
      for (const redirect of ['manual', 'error']) {
        const request = new Request(url, {mode: 'no-cors', redirect});
        assert(request.redirect === redirect, 'Request construction must succeed');
        await rejected(fetch(request.clone()), error => error instanceof TypeError, 'fetch rejects ' + redirect);
        await rejected(fetch(url, {mode: 'no-cors', redirect}), error => error instanceof TypeError, 'URL input rejects ' + redirect);
        await rejected(fetch(new Request(url, {mode: 'no-cors'}), {redirect}), error => error instanceof TypeError, 'RequestInit override rejects ' + redirect);
        const controller = new AbortController();
        const abortReason = {aborted: redirect};
        controller.abort(abortReason);
        await rejected(
          fetch(request, {signal: controller.signal}),
          error => error === abortReason,
          'pre-aborted reason takes precedence'
        );
      }
      return 'ok';
    })()"#;
    vm.eval(&redirect_filter_probe_script(probe, worker))
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            assert!(
                vm.take_pending_subresource_fetch_infos().is_empty(),
                "invalid no-cors requests must not reach interception"
            );
            if vm.eval("redirectFilterResult === 'pending'").unwrap() != "true" {
                break;
            }
            wait_for_one_selected_page_task_executor_test_turn(&mut vm, &loader)
                .await
                .unwrap();
        }
    })
    .await
    .expect("rejected fetch promises should settle without interception");
    assert_eq!(vm.eval("redirectFilterResult").unwrap(), "ok");
}

#[tokio::test]
async fn no_cors_redirect_rejects_window_fetch_before_interception() {
    check_no_cors_redirect_before_interception(false).await;
}

#[tokio::test]
async fn no_cors_redirect_rejects_worker_fetch_before_interception() {
    check_no_cors_redirect_before_interception(true).await;
}
