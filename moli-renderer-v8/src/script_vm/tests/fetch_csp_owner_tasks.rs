use super::*;

#[test]
fn fetch_csp_queues_owner_events_after_rejection_in_enforcement_order() {
    for owner in ["window", "frame.contentWindow", "popup"] {
        for report_only in [false, true] {
            let mut vm = new_storage_page_task_executor_test_vm("https://fetch-csp-tasks.test/");
            vm.set_response_content_security_policies(&["connect-src 'none'".to_owned()]);
            if report_only {
                vm.set_response_content_security_report_only_policies(&[
                    "connect-src 'none'".to_owned()
                ]);
            }
            let immediate = vm
                .eval(&format!(
                    r#"
const frame = document.createElement('iframe'); document.body.appendChild(frame);
const popup = open(); const owner = {owner};
const events = []; const topEvents = [];
let duringCall = true; let rejection = null;
if (owner !== window) {{
  document.addEventListener('securitypolicyviolation', event => topEvents.push(event.type));
}}
owner.document.addEventListener('securitypolicyviolation', event => {{
  events.push({{
    disposition: event.disposition,
    directive: event.effectiveDirective,
    blockedURI: event.blockedURI,
    owner: event.target === owner.document,
    native: event instanceof owner.SecurityPolicyViolationEvent,
    trusted: event.isTrusted,
    duringCall,
  }});
}});
owner.fetch('data:text/plain,blocked').then(
  () => {{ rejection = 'unexpected fulfillment'; }},
  error => {{ rejection = {{
    name: error.name,
    native: error instanceof (owner === popup ? TypeError : owner.TypeError),
    events: events.length,
  }}; }}
);
duringCall = false;
events.length;
"#,
                ))
                .unwrap();
            assert_eq!(immediate, "0", "{owner}, report_only={report_only}");
            let observed: serde_json::Value = serde_json::from_str(
                &vm.eval_after_selected_page_tasks(
                    "JSON.stringify({rejection, events, topEvents})",
                )
                .unwrap(),
            )
            .unwrap();
            let event = |disposition| {
                serde_json::json!({
                    "disposition": disposition,
                    "directive": "connect-src",
                    "blockedURI": "data",
                    "owner": true,
                    "native": true,
                    "trusted": true,
                    "duringCall": false,
                })
            };
            let mut events = vec![event("enforce")];
            if report_only {
                events.push(event("report"));
            }
            assert_eq!(
                observed,
                serde_json::json!({
                    "rejection": {"name": "TypeError", "native": true, "events": 0},
                    "events": events,
                    "topEvents": [],
                }),
                "{owner}, report_only={report_only}",
            );
            vm.eval("popup.close(); frame.remove()").unwrap();
        }
    }
}

#[test]
fn retired_fetch_csp_tasks_cannot_dispatch_after_window_discard() {
    for (owner, discard) in [
        ("frame.contentWindow", "frame.remove()"),
        ("popup", "popup.close()"),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://fetch-csp-retirement.test/");
        vm.eval(&format!(
            r#"
const frame = document.createElement('iframe'); document.body.appendChild(frame);
const popup = open(); const owner = {owner}; const events = [];
const meta = owner.document.createElement('meta');
meta.httpEquiv = 'Content-Security-Policy'; meta.content = "connect-src 'none'";
owner.document.head.appendChild(meta);
owner.document.addEventListener('securitypolicyviolation', event => events.push('old'));
owner.fetch('data:text/plain,blocked').catch(() => {{}});
{discard};
const replacement = document.createElement('iframe'); document.body.appendChild(replacement);
replacement.contentDocument.addEventListener('securitypolicyviolation', event => events.push('new'));
document.addEventListener('securitypolicyviolation', event => events.push('top'));
"#,
        ))
        .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("JSON.stringify(events)")
                .unwrap(),
            "[]",
            "{owner}",
        );
        vm.eval("popup.close(); frame.remove()").unwrap();
    }
}

#[test]
fn connect_csp_dual_policy_events_preserve_each_api_disposition_order() {
    for owner in ["window", "frame.contentWindow"] {
        for (api, action, expected) in [
            (
                "xhr",
                "const client = new owner.XMLHttpRequest(); client.open('GET', '/blocked'); client.send();",
                ["report", "enforce"],
            ),
            (
                "eventsource",
                "const client = new owner.EventSource('/blocked'); client.onerror = () => client.close();",
                ["report", "enforce"],
            ),
            (
                "websocket",
                "const client = new owner.WebSocket('wss://connect-csp-order.test/blocked'); client.onerror = () => {};",
                ["enforce", "report"],
            ),
            (
                "websocketstream",
                "const client = new owner.WebSocketStream('wss://connect-csp-order.test/blocked'); client.opened.catch(() => {}); client.closed.catch(() => {});",
                ["enforce", "report"],
            ),
        ] {
            let mut vm = new_storage_page_task_executor_test_vm("https://connect-csp-order.test/");
            vm.set_response_content_security_policies(&["connect-src 'none'".to_owned()]);
            vm.set_response_content_security_report_only_policies(&[
                "connect-src 'none'".to_owned()
            ]);
            let immediate = vm.eval(&format!(r#"
const frame = document.createElement('iframe'); document.body.appendChild(frame);
const owner = {owner};
const events = []; const first = []; const topEvents = [];
let duringCall = true;
owner.document.addEventListener('securitypolicyviolation', event => {{
    events.push([event.disposition, duringCall, event.target === owner.document]);
}});
owner.document.addEventListener('securitypolicyviolation', event => first.push(event.disposition), {{once:true}});
if (owner !== window) document.addEventListener('securitypolicyviolation', event => topEvents.push(event.disposition));
{action}
duringCall = false;
events.length;
"#)).unwrap_or_else(|error| panic!("{owner}, {api}: {error}"));
            assert_eq!(immediate, "0", "{owner}, {api}");
            let observed: serde_json::Value = serde_json::from_str(
                &vm.eval_after_selected_page_tasks("JSON.stringify({events, first, topEvents})")
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                observed,
                serde_json::json!({
                    "events": [[expected[0], false, true], [expected[1], false, true]],
                    "first": [expected[0]],
                    "topEvents": [],
                }),
                "{owner}, {api}"
            );
            vm.eval("frame.remove()").unwrap();
        }
    }
}

#[test]
fn connect_csp_header_report_precedes_later_meta_enforcement() {
    for owner in ["window", "frame.contentWindow"] {
        for (api, action) in [
            ("fetch", "owner.fetch('/blocked').catch(() => {});"),
            (
                "xhr",
                "const client = new owner.XMLHttpRequest(); client.open('GET', '/blocked'); client.send();",
            ),
            (
                "eventsource",
                "const client = new owner.EventSource('/blocked'); client.onerror = () => client.close();",
            ),
            (
                "websocket",
                "const client = new owner.WebSocket('wss://mixed-csp-order.test/blocked'); client.onerror = () => {};",
            ),
            (
                "websocketstream",
                "const client = new owner.WebSocketStream('wss://mixed-csp-order.test/blocked'); client.opened.catch(() => {}); client.closed.catch(() => {});",
            ),
        ] {
            let mut vm = new_storage_page_task_executor_test_vm("https://mixed-csp-order.test/");
            vm.set_response_content_security_report_only_policies(&[
                "connect-src 'none'; report-uri /report".to_owned(),
            ]);
            let immediate = vm.eval(&format!(r#"
const frame = document.body.appendChild(document.createElement('iframe'));
const owner = {owner};
const meta = owner.document.createElement('meta');
meta.httpEquiv = 'Content-Security-Policy'; meta.content = "connect-src 'none'; img-src 'none'";
owner.document.head.appendChild(meta);
const events = [], first = []; let duringCall = true;
owner.document.addEventListener('securitypolicyviolation', event => {{
  events.push([event.disposition, event.originalPolicy, duringCall, event.isTrusted, event.target === owner.document]);
}});
owner.document.addEventListener('securitypolicyviolation', event => first.push(event.disposition), {{once:true}});
{action}
duringCall = false; events.length;
"#)).unwrap_or_else(|error| panic!("{owner}, {api}: {error}"));
            assert_eq!(immediate, "0", "{owner}, {api}");
            let observed: serde_json::Value = serde_json::from_str(
                &vm.eval_after_selected_page_tasks("JSON.stringify({events, first})")
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                observed,
                serde_json::json!({
                    "events": [
                        ["report", "connect-src 'none'; report-uri /report", false, true, true],
                        ["enforce", "connect-src 'none'; img-src 'none'", false, true, true],
                    ],
                    "first": ["report"],
                }),
                "{owner}, {api}"
            );
        }
    }
}

#[test]
fn connect_csp_keeps_each_response_and_meta_policy_violation() {
    for response_blocks in [false, true] {
        let mut vm =
            new_storage_page_task_executor_test_vm("https://multiple-connect-policies.test/");
        let response = if response_blocks {
            "connect-src 'none'; script-src-attr 'none'"
        } else {
            "connect-src https: wss:"
        };
        let reports = [
            "connect-src 'none'; img-src 'none'",
            "connect-src 'none'; object-src 'none'",
        ];
        let meta = [
            "connect-src 'none'; font-src 'none'",
            "connect-src 'none'; media-src 'none'",
        ];
        vm.set_response_content_security_policies(&[response.to_owned()]);
        vm.set_response_content_security_report_only_policies(&reports.map(str::to_owned));
        vm.eval(&format!(r#"
const events = [], first = [];
for (const content of {}) {{
  const meta = document.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy'; meta.content = content;
  document.head.appendChild(meta);
}}
document.addEventListener('securitypolicyviolation', event => events.push([event.disposition, event.originalPolicy]));
document.addEventListener('securitypolicyviolation', event => first.push(event.disposition), {{once:true}});
fetch('/blocked').catch(() => {{}});
"#, serde_json::to_string(&meta).unwrap())).unwrap();
        let observed: serde_json::Value = serde_json::from_str(
            &vm.eval_after_selected_page_tasks("JSON.stringify({events, first})")
                .unwrap(),
        )
        .unwrap();
        let mut expected = Vec::new();
        if response_blocks {
            expected.push(("enforce", response));
        }
        expected.extend(reports.into_iter().map(|policy| ("report", policy)));
        expected.extend(meta.into_iter().map(|policy| ("enforce", policy)));
        assert_eq!(
            observed,
            serde_json::json!({
                "events": expected,
                "first": [if response_blocks { "enforce" } else { "report" }],
            }),
            "response_blocks={response_blocks}"
        );
    }
}
