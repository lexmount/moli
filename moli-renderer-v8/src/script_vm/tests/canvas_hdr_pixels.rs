use super::*;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "current_thread")]
async fn canvas_hdr_pixels_preserve_native_precision_and_color_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-hdr.test/");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!(r#"(async () => {{
        const probe={};
        const realms=[globalThis,document.getElementById('child').contentWindow],checks=[];
        for(let owner=0;owner<2;owner++)for(let callee=0;callee<2;callee++)
            for(const kind of ['element','offscreen'])
                checks.push(...await probe(realms[owner],realms[callee],kind,owner+'-'+callee+'-'+kind));
        globalThis.__hdrResults=checks;
    }})().catch(error=>globalThis.__hdrResults=String(error));"#,
        include_str!("canvas_hdr_pixels.js"))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while vm.eval("typeof __hdrResults").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(Instant::now() < deadline, "HDR pixel contract timed out");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        vm.eval("Array.isArray(__hdrResults)").unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__hdrResults)").unwrap()
    );
    assert_eq!(vm.eval("__hdrResults.length").unwrap(), "272");
    assert_eq!(
        vm.eval("JSON.stringify(__hdrResults.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}
