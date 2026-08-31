use super::*;

#[tokio::test(flavor = "current_thread")]
async fn initial_navigator_materializes_from_the_committed_document_authority_seed() {
    const USER_AGENT: &str = "Moli-Initial-Document-Navigator/1.0";

    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_user_agent(USER_AGENT);
    let loader = ResourceRequestClient::new(&fetch_config).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://lazy-window-initial-seed.test/", &loader);

    assert_eq!(
        default_surface_state(&mut vm),
        (false, false, false),
        "installing the initial Document authority must not eagerly materialize Navigator"
    );
    assert_eq!(
        vm.eval("navigator.userAgent")
            .expect("initial Navigator should materialize from its Document authority"),
        USER_AGENT
    );
    assert_eq!(
        vm.eval("JSON.stringify(navigator.userAgentData.toJSON())")
            .expect("UA-only override should not retain default client-hint metadata"),
        r#"{"brands":[],"mobile":false,"platform":""}"#
    );
}

#[test]
fn replacing_document_transport_keeps_wire_and_navigator_identities_distinct() {
    const WIRE_USER_AGENT: &str = "Moli-Wire-Identity/1.0";
    const NAVIGATOR_USER_AGENT: &str = "Moli-Navigator-Identity/1.0";

    let mut vm = new_storage_test_vm("https://navigator-transport-split.test/");
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_user_agent(WIRE_USER_AGENT);
    let loader = ResourceRequestClient::new(&fetch_config).expect("loader");
    let navigator_identity =
        moli_browser_profile::BrowserIdentityProfile::new(NAVIGATOR_USER_AGENT, "fr-FR");

    let document_loader =
        vm.replace_document_resource_runtime_with_navigator_identity(&loader, &navigator_identity);

    assert_eq!(
        document_loader
            .request_client()
            .browser_identity()
            .user_agent(),
        WIRE_USER_AGENT,
        "the replacement transport must keep the browser-side request identity"
    );
    assert_eq!(
        vm.eval("navigator.userAgent")
            .expect("Navigator should expose the renderer identity"),
        NAVIGATOR_USER_AGENT
    );
    assert_eq!(
        vm.eval("navigator.language")
            .expect("Navigator language should expose the renderer identity"),
        "fr-FR"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn navigator_identity_seed_keeps_window_metadata_and_network_profile_coherent() {
    const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36";

    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_user_agent(USER_AGENT);
    fetch_config.push_default_request_header("Accept-Language", "fr-CA,fr;q=0.8,en;q=0.5");
    let loader = ResourceRequestClient::new(&fetch_config).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://navigator-identity-seed.test/", &loader);

    assert_eq!(
        loader.browser_identity().sec_ch_ua_value().as_deref(),
        Some("\"Chromium\";v=\"146\", \"Not-A.Brand\";v=\"24\", \"Google Chrome\";v=\"146\"")
    );
    assert_eq!(loader.browser_identity().languages(), ["fr-CA", "fr", "en"]);

    vm.eval(
        r#"
        (() => {
          const ua = navigator.userAgentData;
          globalThis.__navigatorIdentitySeedProbe = {
            base: {
              userAgent: navigator.userAgent,
              language: navigator.language,
              languages: Array.from(navigator.languages),
              brands: ua.brands,
              json: ua.toJSON()
            }
          };
          ua.getHighEntropyValues([
            "architecture",
            "fullVersionList",
            "formFactors",
            "uaFullVersion"
          ]).then((high) => {
            globalThis.__navigatorIdentitySeedProbe.high = high;
          });
        })()
        "#,
    )
    .expect("custom Navigator identity should evaluate");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__navigatorIdentitySeedProbe)")
            .expect("custom Navigator identity promise should settle"),
        format!(
            r#"{{"base":{{"userAgent":"{USER_AGENT}","language":"fr-CA","languages":["fr-CA","fr","en"],"brands":[{{"brand":"Chromium","version":"146"}},{{"brand":"Not-A.Brand","version":"24"}},{{"brand":"Google Chrome","version":"146"}}],"json":{{"brands":[{{"brand":"Chromium","version":"146"}},{{"brand":"Not-A.Brand","version":"24"}},{{"brand":"Google Chrome","version":"146"}}],"mobile":false,"platform":"Windows"}}}},"high":{{"architecture":"x86","brands":[{{"brand":"Chromium","version":"146"}},{{"brand":"Not-A.Brand","version":"24"}},{{"brand":"Google Chrome","version":"146"}}],"formFactors":["Desktop"],"fullVersionList":[{{"brand":"Chromium","version":"146.1.2.3"}},{{"brand":"Not-A.Brand","version":"24.0.0.0"}},{{"brand":"Google Chrome","version":"146.1.2.3"}}],"mobile":false,"platform":"Windows","uaFullVersion":"146.1.2.3"}}}}"#
        )
    );
}

#[test]
fn child_default_and_isolated_navigator_materialize_from_the_loader_seed() {
    const USER_AGENT: &str = "Moli-Lazy-Child-Navigator/1.0";

    let mut vm = new_storage_test_vm("https://lazy-window-child-seed.test/");
    let mut fetch_config = moli_fetch::FetchConfig::default();
    fetch_config.set_user_agent(USER_AGENT);
    let loader = ResourceRequestClient::new(&fetch_config).expect("loader");
    vm.replace_document_resource_runtime(&loader);

    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          frame.id = "lazy-window-child-seed-frame";
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })()
        "#,
    )
    .expect("child frame should be created");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child Navigator seed");
    let frame_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.context_id == child_context_id)
        .and_then(|realm| realm.frame_id)
        .expect("child frame id");
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "lazy-window-child-seed-isolated", false)
        .expect("child isolated world should be created");

    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (false, false, false)
    );
    assert_eq!(
        isolated_surface_state(&mut vm, isolated_context_id),
        (false, false, false)
    );

    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const child =
                document.getElementById("lazy-window-child-seed-frame").contentWindow;
              return JSON.stringify({
                userAgent: child.navigator.userAgent,
                realm:
                  Object.getPrototypeOf(child.navigator) === child.Navigator.prototype
              });
            })()
            "#,
        )
        .expect("default child Navigator should materialize from its seed"),
        format!(r#"{{"userAgent":"{USER_AGENT}","realm":true}}"#)
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            r#"
            JSON.stringify({
              userAgent: navigator.userAgent,
              realm: Object.getPrototypeOf(navigator) === Navigator.prototype
            })
            "#,
        )
        .expect("isolated child Navigator should materialize from its seed"),
        format!(r#"{{"userAgent":"{USER_AGENT}","realm":true}}"#)
    );
    assert_eq!(
        child_surface_state(&mut vm, child_context_id),
        (true, false, false)
    );
    assert_eq!(
        isolated_surface_state(&mut vm, isolated_context_id),
        (true, false, false)
    );
    assert_eq!(
        default_surface_state(&mut vm),
        (false, false, false),
        "child Navigator access must not materialize the top Window surface"
    );
}
