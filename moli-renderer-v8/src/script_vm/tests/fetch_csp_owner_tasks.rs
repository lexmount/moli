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
