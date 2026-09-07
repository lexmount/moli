use super::*;
use moli_page_types::NavigatorOverrides;

#[test]
fn native_navigator_override_clear_restores_native_sources() {
    let mut vm = new_storage_test_vm("https://navigator-overrides.test/");
    let native_touch_points = vm.eval("navigator.maxTouchPoints").unwrap();

    vm.set_network_offline(true);
    vm.set_navigator_overrides_and_sync_surface(&NavigatorOverrides {
        online: Some(true),
        max_touch_points: Some(5),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        vm.eval("[navigator.onLine, navigator.maxTouchPoints].join('|')")
            .unwrap(),
        "true|5"
    );

    vm.set_network_offline(false);
    vm.set_navigator_overrides_and_sync_surface(&NavigatorOverrides {
        online: Some(false),
        max_touch_points: Some(0),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        vm.eval("[navigator.onLine, navigator.maxTouchPoints].join('|')")
            .unwrap(),
        "false|0"
    );

    vm.set_network_offline(true);
    vm.set_navigator_overrides_and_sync_surface(&NavigatorOverrides::default())
        .unwrap();
    assert_eq!(
        vm.eval("[navigator.onLine, navigator.maxTouchPoints].join('|')")
            .unwrap(),
        format!("false|{native_touch_points}")
    );
    vm.set_network_offline(false);
    assert_eq!(
        vm.eval("navigator.onLine").unwrap(),
        "true",
        "clearing must restore the live network source, not a frozen default"
    );
}
