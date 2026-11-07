use super::*;
#[tokio::test]
async fn child_javascript_url_preserves_committed_navigation_entry_identities() {
    for action in [
        "src",
        "attribute",
        "href",
        "window-location",
        "replace",
        "form",
        "hyperlink",
    ] {
        let server = StaticHttpServer::spawn_with_bodies(vec![
            "<!doctype html><body>initial".into(),
            "<!doctype html><body>next".into(),
        ])
        .await;
        let loader = static_http_loader([]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(server.base_url().as_str(), &loader);
        vm.eval(&format!(
            r#"
            globalThis.loads = 0;
            globalThis.executions = 0;
            const frame = document.createElement('iframe');
            frame.name = 'javascript-history-target';
            frame.onload = () => ++loads;
            frame.src = {:?};
            document.body.append(frame);
            const snapshot = entry => ({{key:entry.key, id:entry.id, url:entry.url, index:entry.index}});
            "#,
            server.base_url().join("initial").unwrap().as_str(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "1",
            "initial history document loaded",
        )
        .await;
        vm.eval(
            "const first = snapshot(frame.contentWindow.navigation.currentEntry); \
             frame.contentWindow.navigation.navigate('?next');",
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "2",
            "cross-document navigation committed",
        )
        .await;
        assert_eq!(
            vm.eval(
                r#"JSON.stringify((() => {
                  const entries=frame.contentWindow.navigation.entries();
                  return [entries.length,entries[0].key===first.key,entries[0].id===first.id,entries[0].url===first.url];
                })())"#,
            )
            .unwrap(),
            "[2,true,true,true]",
            "cross-document predecessor: {action}",
        );
        let navigation = match action {
            "src" => "frame.src=code",
            "attribute" => "frame.setAttribute('src',code)",
            "href" => "frame.contentWindow.location.href=code",
            "window-location" => "frame.contentWindow.location=code",
            "replace" => "frame.contentWindow.location.replace(code)",
            "form" => {
                "const form=document.createElement('form');form.target=frame.name;form.action=code;document.body.append(form);form.submit();form.remove()"
            }
            "hyperlink" => {
                "const link=document.createElement('a');link.target=frame.name;link.href=code;document.body.append(link);link.click();link.remove()"
            }
            _ => unreachable!(),
        };
        vm.eval(&format!(
            r#"
            const before=snapshot(frame.contentWindow.navigation.currentEntry);
            const beforeDocument=frame.contentDocument;
            const runJavascript=code=>{{{navigation}}};
            runJavascript('javascript:parent.executions++;void 0');
            "#,
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(executions)",
            "1",
            "non-string javascript completion executed",
        )
        .await;
        assert_eq!(
            vm.eval(
                "JSON.stringify([loads,frame.contentDocument===beforeDocument, \
                 JSON.stringify(snapshot(frame.contentWindow.navigation.currentEntry))===JSON.stringify(before)])",
            )
            .unwrap(),
            "[2,true,true]",
            "non-string completion: {action}",
        );
        vm.eval("runJavascript(\"javascript:'<!doctype html><body>replacement'\")")
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "3",
            "javascript string replaced the document",
        )
        .await;
        assert_eq!(
            vm.eval(
                r#"JSON.stringify((() => {
                  const nav=frame.contentWindow.navigation,entries=nav.entries();
                  return [entries.length,entries[0].key===first.key,entries[0].id===first.id,entries[0].url===first.url,
                    entries[1]===nav.currentEntry,nav.currentEntry.key===before.key,nav.currentEntry.url===before.url,
                    nav.currentEntry.id!==before.id,frame.contentDocument!==beforeDocument,frame.contentDocument.body.textContent];
                })())"#,
            )
            .unwrap(),
            r#"[2,true,true,true,true,true,true,true,true,"replacement"]"#,
            "string completion: {action}",
        );
        assert_eq!(server.finish_targets().await, ["/initial", "/initial?next"]);
    }
}

#[tokio::test]
async fn child_javascript_url_uses_history_committed_during_script_execution() {
    for action in ["src", "href"] {
        for completion in ["string", "void", "throw"] {
            let server =
                StaticHttpServer::spawn_with_bodies(vec!["<!doctype html><body>initial".into()])
                    .await;
            let loader = static_http_loader([]);
            let mut vm = new_storage_page_task_executor_test_vm_with_loader(
                server.base_url().as_str(),
                &loader,
            );
            vm.eval(&format!(
                r#"
                globalThis.loads=0;
                const frame=document.createElement('iframe');
                frame.onload=()=>++loads;
                frame.src={:?};
                document.body.append(frame);
                "#,
                server.base_url().join("initial").unwrap().as_str(),
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(loads)",
                "1",
                "history-changing javascript target loaded",
            )
            .await;
            let result = match completion {
                "string" => "'<!doctype html><body>replacement'",
                "void" => "void 0",
                "throw" => "throw new Error('expected javascript URL failure')",
                _ => unreachable!(),
            };
            let code = format!(
                "javascript:history.pushState(null,'','#inside'); \
                 const entry=navigation.currentEntry; \
                 parent.scriptEntry={{key:entry.key,id:entry.id,url:entry.url}}; {result}",
            );
            let navigation = if action == "src" {
                "frame.src=code"
            } else {
                "frame.contentWindow.location.href=code"
            };
            vm.eval(&format!(
                r#"
                const originalDocument=frame.contentDocument;
                const first=frame.contentWindow.navigation.currentEntry;
                const original={{key:first.key,id:first.id,url:first.url}};
                globalThis.scriptEntry=null;
                globalThis.settled=false;
                const code={code:?};
                {navigation};
                requestIdleCallback(()=>settled=true);
                "#,
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(settled)",
                "true",
                "history-changing javascript URL settled",
            )
            .await;
            assert_eq!(
                vm.eval(&format!(
                    r#"JSON.stringify((() => {{
                      const nav=frame.contentWindow.navigation,entries=nav.entries();
                      return [entries.length,entries[0].key===original.key,entries[0].id===original.id,entries[0].url===original.url,
                        nav.currentEntry.key===scriptEntry.key,nav.currentEntry.url===scriptEntry.url,
                        (nav.currentEntry.id!==scriptEntry.id)==={replaced},
                        (frame.contentDocument!==originalDocument)==={replaced},
                        frame.contentDocument.URL===scriptEntry.url,loads];
                    }})())"#,
                    replaced = completion == "string",
                ))
                .unwrap(),
                format!(
                    "[2,true,true,true,true,true,true,true,true,{}]",
                    if completion == "string" { 2 } else { 1 },
                ),
                "action={action}; completion={completion}",
            );
            assert_eq!(server.finish_targets().await, ["/initial"]);
        }
    }
}
