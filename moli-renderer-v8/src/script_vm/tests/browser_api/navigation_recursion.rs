use super::*;

#[tokio::test]
async fn recursive_child_navigation_stops_without_leaving_load_blocked() {
    for api in ["navigation", "location"] {
        let script = format!(
            r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
const target = new URL('#repeat', location.href).href;
if ({api:?} === 'navigation') frame.contentWindow.navigation.navigate(target);
else frame.contentWindow.location.href = target;
"#
        );
        let body = format!("<!doctype html><body><script>{script}</script>");
        let server = StaticHttpServer::spawn_with_bodies(vec![body]).await;
        let url = server.base_url().join("recursive").unwrap();
        let loader = static_http_loader([]);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url.as_str(), &loader);
        vm.eval(&script).unwrap();
        // The initial about:blank is already complete before the request runs.
        // Wait for the response script to create its nested frame as well.
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            r#"(() => {
const child = document.querySelector('iframe').contentDocument;
return String(child.querySelector('iframe') !== null && child.readyState === 'complete');
})()"#,
            "true",
            api,
        )
        .await;
        assert_eq!(vm.eval(r#"(() => {
const outer = document.querySelector('iframe');
const nested = outer.contentDocument.querySelector('iframe');
return JSON.stringify({outer: outer.contentWindow.location.hash, nested: nested.contentWindow.location.href, ready: nested.contentDocument.readyState});
})()"#).unwrap(), r##"{"outer":"#repeat","nested":"about:blank","ready":"complete"}"##, "{api}");
        assert_eq!(server.finish_targets().await, ["/recursive"], "{api}");
    }
}
