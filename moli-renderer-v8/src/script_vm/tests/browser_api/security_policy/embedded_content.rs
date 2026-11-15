use super::*;

#[tokio::test]
async fn embedded_object_csp_uses_resource_document_before_mime_selection() {
    for (owner, report_only) in [
        ("window", false),
        ("frame.contentWindow", false),
        ("popup", false),
        ("window", true),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://object-policy.test/");
        if report_only {
            vm.set_response_content_security_report_only_policies(
                &["object-src 'none'".to_owned()],
            );
        }
        vm.eval(&format!(r#"
            const frame = document.createElement('iframe'); document.body.appendChild(frame);
            const popup = open(); const owner = {owner}; const doc = owner.document;
            globalThis.objectReports = [];
            doc.addEventListener('securitypolicyviolation', event => objectReports.push({{
                directive: event.effectiveDirective,
                disposition: event.disposition,
                targetIsDocument: event.target === doc,
                documentURI: event.documentURI
            }}));
            if (!{report_only}) {{
                const meta = doc.createElement('meta');
                meta.httpEquiv = 'Content-Security-Policy';
                meta.content = "object-src 'none'; frame-src *; img-src *";
                doc.head.appendChild(meta);
            }}
            globalThis.objects = ['object', 'embed', 'object'].map((name, index) => {{
                const element = doc.createElement(name);
                if (index < 2) element.type = 'image/png';
                element.setAttribute(name === 'object' ? 'data' : 'src',
                    index < 2 ? 'https://object-policy.test/image.png' : 'data:text/html,<p>document</p>');
                doc.body.appendChild(element);
                // Discovery and named-property updates keep the same request.
                for (let i = 0; i < 3; ++i) {{ element.id = 'object-' + index + '-' + i; void element.contentDocument; }}
                return element;
            }});
            globalThis.immediateObjectReports = objectReports.length;
        "#)).unwrap();
        assert_eq!(vm.eval("String(immediateObjectReports)").unwrap(), "0");
        inline_tasks::drain_inline_csp_page_tasks_for_test(&mut vm).await;
        let reports: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(objectReports)").unwrap()).unwrap();
        assert_eq!(
            reports.as_array().unwrap().len(),
            3,
            "{owner}/{report_only}: {reports}"
        );
        for report in reports.as_array().unwrap() {
            assert_eq!(report["directive"], "object-src");
            assert_eq!(
                report["disposition"],
                if report_only { "report" } else { "enforce" }
            );
            assert_eq!(
                report["targetIsDocument"], true,
                "{owner}/{report_only}: {report}"
            );
        }
        if !report_only {
            assert_eq!(
                vm.eval("objects[2].contentDocument === null").unwrap(),
                "true"
            );
        }
        vm.eval("popup.close(); frame.remove()").unwrap();
    }
}

#[tokio::test]
async fn embedded_object_csp_keeps_admission_until_resource_selection_changes() {
    let mut vm = new_storage_page_task_executor_test_vm("https://object-policy-admission.test/");
    vm.eval(r#"
        globalThis.reports = [];
        document.addEventListener('securitypolicyviolation', event => reports.push(event.effectiveDirective));
        const object = document.createElement('object');
        object.data = 'data:text/html,<p>original</p>';
        document.body.appendChild(object);
        globalThis.originalDocument = object.contentDocument;
        const meta = document.createElement('meta');
        meta.httpEquiv = 'Content-Security-Policy';
        meta.content = "object-src 'none'";
        document.head.appendChild(meta);
        object.id = 'same-resource';
        globalThis.existingResourceRetained = object.contentDocument === originalDocument;
        object.data = 'data:text/html,<p>replacement</p>';
        globalThis.replacementBlocked = object.contentDocument === null;
        for (let i = 0; i < 3; ++i) {object.id = 'still-blocked-' + i; void object.contentDocument;}
        object.remove(); document.body.appendChild(object);
        globalThis.reinsertedResourceBlocked = object.contentDocument === null;
    "#).unwrap();
    assert_eq!(vm.eval("JSON.stringify([originalDocument !== null, existingResourceRetained, replacementBlocked, reinsertedResourceBlocked])").unwrap(), "[true,true,true,true]");
    inline_tasks::drain_inline_csp_page_tasks_for_test(&mut vm).await;
    assert_eq!(
        vm.eval("JSON.stringify(reports)").unwrap(),
        r#"["object-src","object-src"]"#
    );
}

#[tokio::test]
async fn embedded_object_csp_allows_object_sources_independently_of_frame_sources_and_bypass() {
    for (policy, bypass) in [
        ("object-src about:; frame-src 'none'", false),
        ("object-src 'none'; frame-src 'none'", true),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://object-policy-allowed.test/");
        vm.set_response_content_security_policies(&[policy.to_owned()]);
        vm.set_bypass_content_security_policy(bypass);
        vm.eval(r#"
            globalThis.reports = [];
            document.addEventListener('securitypolicyviolation', event => reports.push(event.effectiveDirective));
            globalThis.object = document.createElement('object');
            object.data = 'about:blank#object';
            document.body.appendChild(object);
        "#).unwrap();
        assert_eq!(vm.eval("object.contentDocument !== null").unwrap(), "true");
        inline_tasks::drain_inline_csp_page_tasks_for_test(&mut vm).await;
        assert_eq!(
            vm.eval("JSON.stringify(reports)").unwrap(),
            "[]",
            "{policy}/{bypass}"
        );
    }
}
