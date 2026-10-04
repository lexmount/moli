use super::service_worker_drain::drain_service_worker_test_turn;
use super::*;

#[tokio::test]
async fn video_codec_support_queries_convert_clone_and_reject_in_window_child_and_worker_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://codec-support.test/",
            &loader,
        );
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = include_str!("video_codec_support.js");
    vm.eval(&format!("({source}).then(value => globalThis.__codecSupport = value, error => globalThis.__codecSupport = String(error));")).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while vm.eval("globalThis.__codecSupport !== undefined").unwrap() != "true" {
            drain_service_worker_test_turn(&mut vm, &browser_context_runtime, &loader).await;
        }
    })
    .await
    .expect("codec support probes should settle");
    assert_eq!(vm.eval("globalThis.__codecSupport").unwrap(), "ok");
}
