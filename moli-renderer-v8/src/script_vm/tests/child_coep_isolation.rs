use super::*;

#[tokio::test(flavor = "current_thread")]
async fn coep_only_frames_inherit_the_top_level_group_and_keep_container_denials() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/html\r\nCross-Origin-Embedder-Policy: require-corp\r\n",
        |_, _| "<!doctype html><title>COEP-only child</title>".into(),
    )
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let base = server.base_url();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    let headers = vec![
        ("Cross-Origin-Opener-Policy".into(), b"same-origin".to_vec()),
        (
            "Cross-Origin-Embedder-Policy".into(),
            b"require-corp".to_vec(),
        ),
    ];
    vm.set_main_navigation_policy_container(
        crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
            &headers, &base,
        ),
    );
    vm.eval(r#"
        const html = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || html.appendChild(document.createElement('body'));
        globalThis.coepChildrenLoaded = 0;
        for (const denied of [false, true]) {
            const frame = document.createElement('iframe');
            frame.id = denied ? 'denied-coep' : 'allowed-coep';
            if (denied) frame.setAttribute('allow', "cross-origin-isolated 'none'");
            frame.onload = () => coepChildrenLoaded++;
            frame.src = denied ? '/denied' : '/allowed';
            body.appendChild(frame);
        }
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(coepChildrenLoaded)",
        "2",
        "COEP-only child documents",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(['allowed-coep','denied-coep'].map(id=>document.getElementById(id).contentWindow.crossOriginIsolated))").unwrap(), "[true,false]");
    assert_eq!(server.finish().await.len(), 2);
}

#[test]
fn dip_isolation_does_not_create_a_coop_coep_group() {
    let url = Url::parse("https://dip-mode.test/").unwrap();
    let headers = vec![(
        "Document-Isolation-Policy".into(),
        b"isolate-and-require-corp".to_vec(),
    )];
    let policy = crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
        &headers, &url,
    );
    assert!(policy.cross_origin_isolated);
    assert!(!policy.coop_coep_isolation);
}
