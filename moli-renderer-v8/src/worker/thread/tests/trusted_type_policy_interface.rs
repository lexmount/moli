use super::*;

#[tokio::test]
async fn worker_trusted_type_policy_exposes_shared_native_prototype() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const first = trustedTypes.createPolicy('first', {createHTML: value => value});
        const second = trustedTypes.createPolicy('second', {createHTML: value => value});
        let missing;
        try { first.createScript('x'); } catch(error) { missing = error instanceof TypeError; }
        postMessage({constructor:typeof TrustedTypePolicy, brand:first instanceof TrustedTypePolicy,
            prototype:Object.getPrototypeOf(first) === TrustedTypePolicy.prototype,
            shared:first.createHTML === second.createHTML,
            own:Object.hasOwn(first,'createHTML'), name:first.name,
            value:trustedTypes.isHTML(first.createHTML('ok')), missing});
        close();
    "#
        .into(),
        "https://trusted-policy-worker.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv())
        .await
        .expect("policy worker timed out")
        .expect("policy worker channel closed");
    let actual: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({"constructor":"function", "brand":true,
        "prototype":true, "shared":true, "own":false, "name":"first", "value":true, "missing":true})
    );
}
