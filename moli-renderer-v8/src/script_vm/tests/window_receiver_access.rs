use super::frame_owner_origin_access::child_context_for_element;
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn borrowed_window_callbacks_check_receiver_access_before_conversion_or_realm_entry() {
    const ROOT: &str = "root.window-receiver.test";
    const CHILD: &str = "child.window-receiver.test";
    let server = StaticHttpServer::spawn_with_responder(
        1,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| "<!doctype html><body><script>top.postMessage('ready', '*')</script>".to_owned(),
    )
    .await;
    let loader = static_http_loader([server.resolve_entry(ROOT), server.resolve_entry(CHILD)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        server.url_for_host(ROOT, "/root").as_str(),
        &loader,
    );
    vm.eval(&format!(r#"
        globalThis.ready = false;
        onmessage = event => {{ if (event.data === 'ready') ready = true; }};
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        body.appendChild(Object.assign(document.createElement('iframe'), {{id: 'foreign', src: {:?}}}));
    "#, server.url_for_host(CHILD, "/child").as_str())).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(ready)",
        "true",
        "the foreign Window must have a committed Realm",
    )
    .await;
    let child = child_context_for_element(&mut vm, "foreign");
    let result = vm.eval_in_child_default_context(child, r#"
        const probe = callback => {
          try { callback(); return 'exposed'; }
          catch (error) { return error.name; }
        };
        let conversions = 0;
        const argument = {toString() { ++conversions; return 'all'; }};
        const getters = ['navigator', 'performance', 'customElements', 'screen', 'history', 'indexedDB', 'name', 'origin', 'innerWidth', 'locationbar'];
        JSON.stringify({
          getters: getters.map(key => probe(() => Object.getOwnPropertyDescriptor(window, key).get.call(top))),
          methods: [
            probe(() => matchMedia.call(top, argument)),
            probe(() => setTimeout.call(top, argument)),
            probe(() => stop.call(top))
          ],
          setters: [
            probe(() => Object.getOwnPropertyDescriptor(window, 'name').set.call(top, argument)),
            probe(() => Object.getOwnPropertyDescriptor(window, 'performance').set.call(top, argument))
          ],
          conversions,
          forgedGetters: ['name', 'origin', 'navigator', 'performance', 'indexedDB'].map(key =>
            probe(() => Object.getOwnPropertyDescriptor(window, key).get.call({}))),
          forgedMethods: [
            probe(() => btoa.call({}, 'x')),
            probe(() => setTimeout.call({}, () => {})),
            probe(() => queueMicrotask.call({}, () => {})),
            probe(() => structuredClone.call({}, {}))
          ],
          allowed: [window.postMessage.call(top, 'allowed', '*') === undefined, top.parent === top, top.frames === top]
        })
    "#).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "getters": vec!["SecurityError"; 10],
            "methods": vec!["SecurityError"; 3],
            "setters": vec!["SecurityError"; 2],
            "conversions": 0,
            "forgedGetters": vec!["TypeError"; 5],
            "forgedMethods": vec!["TypeError"; 4],
            "allowed": [true, true, true]
        })
    );
    for universal in [false, true] {
        let world = vm
            .create_isolated_world(
                if universal {
                    "receiver-universal"
                } else {
                    "receiver-restricted"
                },
                universal,
            )
            .unwrap();
        assert_eq!(
            vm.eval_in_isolated_context(
                world,
                r#"
            (() => {
              const target = document.getElementById('foreign').contentWindow;
              const getter = Object.getOwnPropertyDescriptor(window, 'navigator').get;
              try { getter.call(target); return 'exposed'; }
              catch (error) { return error.name; }
            })()
        "#
            )
            .unwrap(),
            if universal {
                "exposed"
            } else {
                "SecurityError"
            }
        );
    }
    assert_eq!(server.finish().await.len(), 1);
}

#[test]
fn cross_origin_facades_do_not_share_the_creators_security_token() {
    let mut vm = new_storage_test_vm("https://facade-receiver.test/");
    vm.eval(
        r#"
        const frame = document.createElement('iframe');
        frame.src = 'data:text/html,<p>foreign</p>';
        (document.body || document.documentElement || document).appendChild(frame);
    "#,
    )
    .unwrap();
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(vm.eval(r#"
        const foreign = frame.contentWindow;
        const probe = callback => { try { callback(); return 'exposed'; } catch (error) { return error.name; } };
        JSON.stringify([
          probe(() => foreign.arbitrary),
          probe(() => { foreign.arbitrary = 1; }),
          probe(() => Object.getOwnPropertyDescriptor(foreign, 'arbitrary')),
          probe(() => Object.getOwnPropertyDescriptor(window, 'document').get.call(foreign)),
          foreign.parent === window,
          foreign === frame.contentWindow
        ])
    "#).unwrap(), r#"["SecurityError","SecurityError","SecurityError","SecurityError",true,true]"#);
}

#[tokio::test(flavor = "current_thread")]
async fn nested_initial_windows_materialize_for_their_parent_and_preserve_proxy_identity() {
    const ROOT: &str = "root.initial-window.test";
    const PARENT: &str = "parent.initial-window.test";
    let server = StaticHttpServer::spawn_with_responder(
        1,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| {
            r#"<!doctype html><body><iframe id="blank"></iframe>
            <iframe id="srcdoc" srcdoc="<p>inherited</p>"></iframe>
            <iframe id="opaque" sandbox srcdoc="<p>opaque</p>"></iframe>
            <script>top.postMessage('ready', '*')</script>"#
                .to_owned()
        },
    )
    .await;
    let loader = static_http_loader([server.resolve_entry(ROOT), server.resolve_entry(PARENT)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        server.url_for_host(ROOT, "/root").as_str(),
        &loader,
    );
    vm.eval(&format!(r#"
        globalThis.ready = false;
        onmessage = event => {{ if (event.data === 'ready') ready = true; }};
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        body.appendChild(Object.assign(document.createElement('iframe'), {{id: 'parent', src: {:?}}}));
    "#, server.url_for_host(PARENT, "/parent").as_str())).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(ready)",
        "true",
        "the parent document must commit",
    )
    .await;
    vm.eval(
        "globalThis.retainedBlank = document.getElementById('parent').contentWindow[0]; 'retained'",
    )
    .unwrap();
    let parent = child_context_for_element(&mut vm, "parent");
    assert_eq!(vm.eval_in_child_default_context(parent, r#"
        JSON.stringify(['blank', 'srcdoc'].map((id, index) => {
          const frame = document.getElementById(id);
          const child = frames[index];
          return child === frame.contentWindow && frame.contentDocument !== null && child.document === frame.contentDocument && child.parent === window && child.top === top;
        }).concat(document.getElementById('opaque').contentDocument === null))
    "#).unwrap(), "[true,true,true]");
    assert_eq!(
        vm.eval(
            r#"
        JSON.stringify([
          document.getElementById('parent').contentWindow[0] === retainedBlank,
          (() => { try { return retainedBlank.document; } catch(error) { return error.name; } })()
        ])
    "#
        )
        .unwrap(),
        r#"[true,"SecurityError"]"#
    );
    assert_eq!(server.finish().await.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn retained_window_callbacks_cannot_follow_a_foreign_document_after_navigation() {
    const ROOT: &str = "root.retained-callback.test";
    const FOREIGN: &str = "foreign.retained-callback.test";
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| {
            "<!doctype html><body><script>top.postMessage(location.pathname, '*')</script>"
                .to_owned()
        },
    )
    .await;
    let loader = static_http_loader([server.resolve_entry(ROOT), server.resolve_entry(FOREIGN)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        server.url_for_host(ROOT, "/root").as_str(),
        &loader,
    );
    vm.eval(&format!(r#"
        globalThis.loaded = new Set();
        onmessage = event => loaded.add(event.data);
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        body.appendChild(Object.assign(document.createElement('iframe'), {{id: 'target', src: {:?}}}));
    "#, server.url_for_host(ROOT, "/initial").as_str())).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(loaded.has('/initial'))",
        "true",
        "the initial same-origin document must commit",
    )
    .await;
    vm.eval(&format!(r#"
        globalThis.retainedWindow = document.getElementById('target').contentWindow;
        globalThis.retainedGetter = Object.getOwnPropertyDescriptor(retainedWindow, 'navigator').get;
        globalThis.retainedMethod = retainedWindow.matchMedia;
        document.getElementById('target').src = {:?};
    "#, server.url_for_host(FOREIGN, "/foreign").as_str())).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(loaded.has('/foreign'))",
        "true",
        "the foreign document must replace the old Realm",
    )
    .await;
    assert_eq!(vm.eval(r#"
        const probe = f => {try {f(); return 'exposed'} catch(error) {return error.name}};
        let conversions = 0;
        JSON.stringify([
          retainedWindow === document.getElementById('target').contentWindow,
          probe(() => retainedGetter.call(retainedWindow)),
          probe(() => retainedMethod.call(retainedWindow, {toString() {++conversions; return 'all'}})),
          conversions
        ])
    "#).unwrap(), r#"[true,"SecurityError","SecurityError",0]"#);
    assert_eq!(server.finish().await.len(), 2);
}
