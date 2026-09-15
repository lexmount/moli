use super::*;

#[tokio::test]
async fn worker_fetch_csp_violations_preserve_each_call_location() {
    ensure_v8();
    for enforce in [false, true] {
        let source = r#"
const locations = [];
addEventListener('securitypolicyviolation', e => {
  locations.push([e.sourceFile, e.lineNumber, e.columnNumber]);
  if (locations.length === 6) { postMessage(locations); close(); }
});
for (let i = 0; i < 5; i++) { fetch('data:text/plain,blocked').catch(() => {}); }
fetch('data:text/plain,blocked').catch(() => {});
"#;
        let policies = vec!["connect-src 'none'".to_owned()];
        let options = WorkerSpawnOptions::new(
            source.to_owned(),
            "https://app.test/fetch-worker.js?secret#fragment".into(),
        );
        let mut handle = spawn_test_worker_with_options(if enforce {
            options.with_content_security_policies(policies)
        } else {
            options.with_content_security_report_only_policies(policies)
        });
        let locations: Vec<(String, i32, i32)> =
            serde_json::from_str(&recv_post_json(&mut handle).await).unwrap();
        assert_eq!(locations.len(), 6);
        assert!(locations.iter().all(|(url, line, column)| url
            == "https://app.test/fetch-worker.js"
            && *line > 0
            && *column > 0));
        assert!(
            locations[..5]
                .iter()
                .all(|location| location == &locations[0])
        );
        assert_ne!(locations[0].1, locations[5].1);
    }
}

fn assert_initial_worker_auth_network_headers(headers: Option<&[(String, String)]>) {
    let headers = headers.expect("worker auth transport request headers");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("host") && !value.is_empty()),
        "worker auth transport headers should contain Host: {headers:?}"
    );
    assert!(
        headers
            .iter()
            .all(|(name, _)| !name.eq_ignore_ascii_case("authorization")),
        "the browser-visible auth request observation must remain the initial unauthenticated exchange: {headers:?}"
    );
}

mod fetch_network;
mod file_and_xhr;
mod opfs_storage;
mod websockets;
mod worker_globals;
mod worker_storage_security;
