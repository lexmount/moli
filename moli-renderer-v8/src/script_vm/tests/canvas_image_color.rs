use super::*;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "current_thread")]
async fn canvas_image_profiles_preserve_gamut_and_bitmap_color_conversion_options() {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-image/tests/data/color_profiles/Display-P3-BB0000CC.png"
    ));
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-image-color.test/");
    vm.eval(&format!("const probe={}; probe({bytes:?}).then(rows => globalThis.__imageColors=rows, error => globalThis.__imageColors=String(error));",include_str!("canvas_image_color.js"))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while vm.eval("typeof __imageColors").unwrap() == "undefined" {
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap();
        assert!(
            Instant::now() < deadline,
            "image color conversion timed out"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        vm.eval("Array.isArray(__imageColors)").unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__imageColors)").unwrap()
    );
    assert_eq!(vm.eval("__imageColors.length").unwrap(), "48");
    assert_eq!(
        vm.eval("JSON.stringify(__imageColors.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
}
