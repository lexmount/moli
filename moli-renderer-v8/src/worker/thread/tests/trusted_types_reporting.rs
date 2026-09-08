use super::*;

#[tokio::test]
async fn worker_trusted_types_policy_creation_queues_every_violated_policy() {
    ensure_v8();
    let enforced = [
        "trusted-types allowed duplicate reportOnly",
        "trusted-types allowed duplicate reportOnly",
        "trusted-types allowed duplicate reportOnly 'allow-duplicates'",
    ]
    .map(str::to_owned);
    let report_only = [
        "trusted-types allowed duplicate",
        "trusted-types allowed duplicate",
        "trusted-types allowed duplicate 'allow-duplicates'",
    ]
    .map(str::to_owned);
    for enforce in [false, true] {
        let mut expected = Vec::new();
        for (name, count) in [("reportOnly", 3), ("duplicate", 2), ("blocked", 3)] {
            if enforce && name != "reportOnly" {
                expected.extend(
                    enforced[..count]
                        .iter()
                        .map(|policy| serde_json::json!([name, "enforce", policy])),
                );
            }
            expected.extend(
                report_only[..count]
                    .iter()
                    .map(|policy| serde_json::json!([name, "report", policy])),
            );
        }
        let script = format!(
            r#"
const reports = [];
addEventListener('securitypolicyviolation', event => {{
  reports.push([event.sample, event.disposition, event.originalPolicy]);
  if (reports.length === {}) {{ postMessage(reports); close(); }}
}});
const names = ['allowed', 'reportOnly', 'duplicate', 'duplicate', 'blocked'].map(name => {{
  try {{ return trustedTypes.createPolicy(name).name; }}
  catch (error) {{ return error.name; }}
}});
postMessage({{names, reports: reports.length}});
"#,
            expected.len()
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(script, "https://app.test/worker/main.js".into())
                .with_content_security_policies(if enforce { enforced.to_vec() } else { vec![] })
                .with_content_security_report_only_policies(report_only.to_vec()),
        );
        let sync = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        assert_eq!(
            expect_post_json(sync),
            if enforce {
                r#"{"names":["allowed","reportOnly","duplicate","TypeError","TypeError"],"reports":0}"#
            } else {
                r#"{"names":["allowed","reportOnly","duplicate","duplicate","blocked"],"reports":0}"#
            }
        );
        let reports = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        let actual: serde_json::Value = serde_json::from_str(&expect_post_json(reports)).unwrap();
        assert_eq!(actual, serde_json::json!(expected));
    }
}

#[tokio::test]
async fn worker_trusted_types_sink_reports_cover_every_policy_and_report_only_execution() {
    ensure_v8();
    let policies = [
        "require-trusted-types-for 'script'",
        "require-trusted-types-for 'invalid' 'script'",
        "require-trusted-types-for 'script'",
    ]
    .map(str::to_owned);
    for enforce in [false, true] {
        for (operation, error, sample) in [
            (
                "eval('globalThis.ran = true')",
                "EvalError",
                "eval|globalThis.ran = true",
            ),
            (
                "clearTimeout(setTimeout('globalThis.ran = true'))",
                "TypeError",
                "WorkerGlobalScope setTimeout|globalThis.ran = true",
            ),
            (
                "clearInterval(setInterval('globalThis.ran = true'))",
                "TypeError",
                "WorkerGlobalScope setInterval|globalThis.ran = true",
            ),
            (
                "importScripts('data:text/javascript,globalThis.ran=true')",
                "TypeError",
                "WorkerGlobalScope importScripts|data:text/javascript,globalThis.ran=true",
            ),
        ] {
            let expected_count = if enforce { 6 } else { 3 };
            let script = format!(
                r#"
const reports = [];
globalThis.ran = false;
addEventListener('securitypolicyviolation', event => {{
  reports.push([event.sample, event.disposition, event.originalPolicy, event.documentURI,
                event.sourceFile, event.lineNumber > 0, event.columnNumber > 0]);
  if (reports.length === {expected_count}) {{ postMessage(reports); close(); }}
}});
let error = null;
try {{ {operation}; }} catch (e) {{ error = e.name; }}
postMessage({{error, ran, reports: reports.length}});
"#
            );
            let url = "https://app.test/worker/main.js";
            let mut handle = spawn_test_worker_with_options(
                WorkerSpawnOptions::new(script, url.into())
                    .with_content_security_policies(if enforce {
                        policies.to_vec()
                    } else {
                        vec![]
                    })
                    .with_content_security_report_only_policies(policies.to_vec()),
            );
            let sync = timeout(TIMEOUT, handle.recv())
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "{operation}, enforce={enforce}: waiting for synchronous result: {error}"
                    )
                })
                .unwrap();
            let sync: serde_json::Value = serde_json::from_str(&expect_post_json(sync)).unwrap();
            assert_eq!(
                sync,
                serde_json::json!({
                    "error": if enforce { Some(error) } else { None },
                    "ran": !enforce && !operation.starts_with("clear"),
                    "reports": 0,
                })
            );
            let reports = timeout(TIMEOUT, handle.recv()).await
                .unwrap_or_else(|error| panic!("{operation}, enforce={enforce}: waiting for {expected_count} reports: {error}"))
                .unwrap();
            let actual: serde_json::Value =
                serde_json::from_str(&expect_post_json(reports)).unwrap();
            let dispositions = if enforce {
                vec!["enforce", "report"]
            } else {
                vec!["report"]
            };
            let expected: Vec<_> = dispositions
                .into_iter()
                .flat_map(|disposition| {
                    policies.iter().map(move |policy| {
                        serde_json::json!([sample, disposition, policy, url, url, true, true,])
                    })
                })
                .collect();
            assert_eq!(actual, serde_json::json!(expected), "{operation}");
        }
    }
}

#[tokio::test]
async fn worker_trusted_types_report_only_sinks_apply_default_policy_without_false_reports() {
    ensure_v8();
    let policy = "require-trusted-types-for 'script'; trusted-types default named";
    for enforce in [false, true] {
        let expected_count = if enforce { 2 } else { 1 };
        let script = format!(
            r#"
const reports = [];
const calls = [];
addEventListener('securitypolicyviolation', event => {{
  reports.push([event.effectiveDirective, event.sample, event.disposition]);
  if (reports.length === {expected_count}) {{ postMessage(reports); close(); }}
}});
trustedTypes.createPolicy('default', {{
  createScript: (input, type, sink) => {{ calls.push([input, type, sink]); return ''; }},
  createScriptURL: (input, type, sink) => {{
    calls.push([input, type, sink]);
    return 'data:text/javascript,globalThis.imported=true';
  }}
}});
const named = trustedTypes.createPolicy('named', {{createScript: s => s, createScriptURL: s => s}});
importScripts('input');
clearTimeout(setTimeout('input'));
clearInterval(setInterval('input'));
importScripts(named.createScriptURL('data:text/javascript,globalThis.trustedImported=true'));
clearTimeout(setTimeout(named.createScript('')));
clearInterval(setInterval(named.createScript('')));
// A final policy violation provides an event-driven barrier for earlier sink reports.
try {{ trustedTypes.createPolicy('blocked'); }} catch (_) {{}}
postMessage({{calls, imported, trustedImported, reports: reports.length}});
"#
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(script, "https://app.test/worker/main.js".into())
                .with_content_security_policies(if enforce { vec![policy.into()] } else { vec![] })
                .with_content_security_report_only_policies(vec![policy.into()]),
        );
        let sync = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        let sync: serde_json::Value = serde_json::from_str(&expect_post_json(sync)).unwrap();
        assert_eq!(
            sync,
            serde_json::json!({
                "calls": [
                    ["input", "TrustedScriptURL", "WorkerGlobalScope importScripts"],
                    ["input", "TrustedScript", "WorkerGlobalScope setTimeout"],
                    ["input", "TrustedScript", "WorkerGlobalScope setInterval"],
                ],
                "imported": true,
                "trustedImported": true,
                "reports": 0,
            }),
        );
        let reports = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        let reports: serde_json::Value = serde_json::from_str(&expect_post_json(reports)).unwrap();
        let mut expected = Vec::new();
        if enforce {
            expected.push(serde_json::json!(["trusted-types", "blocked", "enforce"]));
        }
        expected.push(serde_json::json!(["trusted-types", "blocked", "report"]));
        assert_eq!(reports, serde_json::json!(expected));
    }
}

#[tokio::test]
async fn worker_trusted_types_reports_precede_later_websocket_csp_reports() {
    ensure_v8();
    let policy = "trusted-types allowed; require-trusted-types-for 'script'; connect-src 'none'";
    let script = r#"
const reports = [];
addEventListener('securitypolicyviolation', event => {
  reports.push([event.effectiveDirective, event.disposition]);
  if (reports.length === 6) { postMessage(reports); close(); }
});
(async () => {
  try { trustedTypes.createPolicy('blocked'); } catch (_) {}
  try { eval(''); } catch (_) {}
  await Promise.resolve();
  new WebSocket('ws://example.test/blocked');
  postMessage(reports.length);
})();
"#;
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(script.into(), "https://app.test/worker/main.js".into())
            .with_content_security_policies(vec![policy.into()])
            .with_content_security_report_only_policies(vec![policy.into()]),
    );
    assert_eq!(recv_post_json(&mut handle).await, "0");
    let reports: serde_json::Value =
        serde_json::from_str(&recv_post_json(&mut handle).await).unwrap();
    assert_eq!(
        reports,
        serde_json::json!([
            ["trusted-types", "enforce"],
            ["trusted-types", "report"],
            ["require-trusted-types-for", "enforce"],
            ["require-trusted-types-for", "report"],
            ["connect-src", "report"],
            ["connect-src", "enforce"],
        ]),
    );
}
