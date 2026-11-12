use super::*;

#[tokio::test]
async fn resource_initiator_geolocation_watch_preserves_registration_across_native_updates() {
    let server = StaticHttpServer::spawn_with_responder(
        4,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::json!("geolocation"),
        setting: "granted".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);
    let registration = base.join("registration.js").unwrap();
    execute_with_source(
        &mut vm,
        r#"
        globalThis.notifications = 0;
        globalThis.watch = navigator.geolocation.watchPosition(
          () => request('success'), () => request('error'));
        function request(kind) {
          const x = new XMLHttpRequest();
          x.open('GET', '/' + (++notifications) + '-' + kind);
          x.send();
        }
        "#,
        &registration,
        &base,
    );
    for (count, latitude) in [(1, None), (2, Some(12.0)), (3, Some(34.0)), (4, None)] {
        if count > 1 {
            vm.set_navigator_overrides_and_sync_surface(&moli_page_types::NavigatorOverrides {
                geolocation: latitude.map(|latitude| {
                    moli_page_types::GeolocationOverride::Position(
                        moli_page_types::GeolocationPositionOverride {
                            latitude,
                            longitude: 56.0,
                            accuracy: 1.0,
                            altitude: None,
                            altitude_accuracy: None,
                            heading: None,
                            speed: None,
                        },
                    )
                }),
                ..Default::default()
            })
            .unwrap();
        }
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(performance.getEntriesByType('resource').length)",
            &count.to_string(),
            "geolocation watch request",
        )
        .await;
    }
    vm.eval("navigator.geolocation.clearWatch(watch);").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(entry => [new URL(entry.name).pathname, entry.initiatorUrl]).sort())").unwrap(),
        serde_json::json!([
            ["/1-error", registration.as_str()], ["/2-success", registration.as_str()],
            ["/3-success", registration.as_str()], ["/4-error", registration.as_str()]
        ]).to_string()
    );
    assert_eq!(server.finish().await.len(), 4);
}
