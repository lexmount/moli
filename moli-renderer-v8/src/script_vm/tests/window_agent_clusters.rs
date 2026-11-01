use super::*;

const DIP: &str = "Document-Isolation-Policy: isolate-and-require-corp\r\n";
const DIP_CREDENTIALLESS: &str = "Document-Isolation-Policy: isolate-and-credentialless\r\n";
const COEP: &str = "Cross-Origin-Embedder-Policy: require-corp\r\n";
const COOP_COEP: &str =
    "Cross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\n";
const DIP_COEP: &str = "Document-Isolation-Policy: isolate-and-require-corp\r\nCross-Origin-Embedder-Policy: require-corp\r\n";
const DENIED_COOP_COEP: &str = "Cross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nPermissions-Policy: cross-origin-isolated=()\r\n";

#[tokio::test(flavor = "current_thread")]
async fn window_access_and_shared_memory_follow_native_agent_clusters() {
    for (root_headers, child_headers, credentialless, allowed, shared, child_isolated) in [
        ("", DIP, false, false, false, true),
        (DIP, "", false, false, false, false),
        (DIP, DIP_CREDENTIALLESS, false, true, true, true),
        (COOP_COEP, COEP, false, true, true, true),
        (DENIED_COOP_COEP, COEP, false, true, false, false),
        (COOP_COEP, DIP_COEP, false, false, true, true),
        ("", COEP, false, true, false, false),
        (COOP_COEP, "", true, true, true, true),
        (DENIED_COOP_COEP, "", true, true, false, false),
    ] {
        let server = StaticHttpServer::spawn_with_raw_responder(1, move |_, _| {
            let body = r#"<!doctype html><body><script>
const attempt = callback => { try { return callback(); } catch (error) { return error.name; } };
onmessage = event => {
  parent.postMessage({kind:'shared', value:event.data.byteLength === 4 ? 'shared' : 'wrong bytes'}, '*');
};
onmessageerror = () => parent.postMessage({kind:'shared', value:'messageerror'}, '*');
parent.postMessage({kind:'ready', isolated:crossOriginIsolated,
  value:attempt(() => parent.document.nodeType === 9),
  location:attempt(() => new URL(parent.location.href).hostname === location.hostname),
  postMessage:typeof parent.postMessage, frames:typeof parent.frames}, '*');
</script>"#;
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{child_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        })
        .await;
        let loader = static_http_loader([server.resolve_entry("localhost")]);
        let root_url = server.url_for_host("localhost", "/root");
        let child_url = server.url_for_host("localhost", "/child");
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(root_url.as_str(), &loader);
        let headers = root_headers
            .lines()
            .map(|line| {
                let (name, value) = line.split_once(':').unwrap();
                (name.to_owned(), value.trim().as_bytes().to_vec())
            })
            .collect::<Vec<_>>();
        vm.set_main_navigation_policy_container(
            crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                &headers, &root_url,
            ),
        );
        vm.eval(&format!(
            r#"
globalThis.clusterReady = null;
globalThis.clusterShared = null;
onmessage = event => {{
  if (event.data.kind === 'ready') clusterReady = event.data;
  if (event.data.kind === 'shared') clusterShared = event.data.value;
}};
const root = document.documentElement || document.appendChild(document.createElement('html'));
const body = document.body || root.appendChild(document.createElement('body'));
body.appendChild(Object.assign(document.createElement('iframe'), {{id:'target', credentialless:{credentialless}, src:{:?}}}));
"#,
            child_url.as_str()
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(clusterReady !== null)",
            "true",
            "the child agent must report through the cross-origin-allowed postMessage surface",
        )
        .await;
        let result = vm
            .eval(
                r#"JSON.stringify((() => {
  const frame = document.getElementById('target');
  const attempt = callback => { try { return callback(); } catch (error) { return error.name; } };
  return {contentDocument:frame.contentDocument !== null,
    read:attempt(() => frame.contentWindow.document.nodeType === 9),
    write:attempt(() => {frame.contentWindow.nativeProbe = true; return true;}),
    location:attempt(() => new URL(frame.contentWindow.location.href).hostname === location.hostname),
    child:clusterReady.value, childLocation:clusterReady.location, childIsolated:clusterReady.isolated,
    postMessage:clusterReady.postMessage, frames:clusterReady.frames};
})())"#,
            )
            .unwrap();
        let expected_access = if allowed {
            serde_json::json!(true)
        } else {
            serde_json::json!("SecurityError")
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::json!({"contentDocument":allowed, "read":expected_access,
                "write":expected_access,"location":expected_access,"child":expected_access,
                "childLocation":expected_access,"childIsolated":child_isolated,
                "postMessage":"function","frames":"object"}),
            "root={root_headers:?}, child={child_headers:?}, credentialless={credentialless}"
        );
        for universal in [false, true] {
            let world = vm
                .create_isolated_world(
                    if universal {
                        "agent-cluster-universal"
                    } else {
                        "agent-cluster-restricted"
                    },
                    universal,
                )
                .unwrap();
            assert_eq!(
                vm.eval_in_isolated_context(
                    world,
                    r#"(() => {
  try { return String(document.getElementById('target').contentWindow.document.nodeType === 9); }
  catch (error) { return error.name; }
})()"#
                )
                .unwrap(),
                if universal || allowed {
                    "true"
                } else {
                    "SecurityError"
                },
                "universal={universal}, root={root_headers:?}, child={child_headers:?}"
            );
        }
        if shared {
            vm.eval("document.getElementById('target').contentWindow.postMessage(new SharedArrayBuffer(4), '*')")
                .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(clusterShared)",
                if allowed { "shared" } else { "messageerror" },
                "shared memory must use the same cluster boundary as DOM access",
            )
            .await;
        }
        assert_eq!(server.finish_targets().await, ["/child"]);
    }
}
