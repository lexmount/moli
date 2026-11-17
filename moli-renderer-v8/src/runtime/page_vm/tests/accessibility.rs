use super::*;

#[tokio::test(flavor = "current_thread")]
async fn implicit_section_role_depends_on_the_computed_accessible_name() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/section.html")?,
        );
        page.vm_mut().eval(r#"
            document.body.innerHTML = `
                <span id="label" hidden>Referenced name</span>
                <span id="empty-label"></span>
                <section id="plain"><p>Plain contents</p></section>
                <section id="unnamed" role="none" aria-hidden="false"><p>Visible body text</p></section>
                <section id="named" role="none" aria-hidden="false" aria-label="News">Body text</section>
                <section id="titled" title="From title">Body text</section>
                <section id="referenced" role="presentation" aria-labelledby="label">Body text</section>
                <section id="empty-reference" aria-labelledby="empty-label">Body text</section>
                <section id="whitespace" aria-label=" &#9; ">Body text</section>
                <section id="foreign-label" role="none" aria-hidden="false">Body text</section>
                <section id="explicit" role="region">Body text</section>
                <section id="override" role="group" aria-label="Explicit group">Body text</section>`;
            document.getElementById('foreign-label').setAttributeNS('urn:test', 'aria-label', 'Foreign label');
        "#)?;
        let nodes = page.accessibility_tree_payloads_for_document(None).expect("full AX tree");
        for (id, role, name) in [
            ("plain", "generic", ""),
            ("unnamed", "generic", ""),
            ("named", "region", "News"),
            ("titled", "region", "From title"),
            ("referenced", "region", "Referenced name"),
            ("empty-reference", "generic", ""),
            ("whitespace", "generic", ""),
            ("foreign-label", "generic", ""),
            ("explicit", "generic", ""),
            ("override", "group", "Explicit group"),
        ] {
            let handle = page.vm().document_runtime.dom_host().element_handle_by_id(id)
                .expect("fixture element");
            let partial = page.accessibility_node_payload_for_live_handle(handle).expect("local AX node");
            assert_eq!(partial["ignored"], false, "{id}");
            assert_eq!(partial["role"]["value"], role, "{id}");
            assert_eq!(partial["name"]["value"].as_str().unwrap_or(""), name, "{id}");
            let full = nodes.iter().find(|node| node["nodeId"] == partial["nodeId"])
                .expect("same AX node in the full tree");
            assert_eq!(full, &partial, "{id}");
        }
        let handle = page.vm().document_runtime.dom_host().element_handle_by_id("referenced")
            .expect("referenced section");
        let before = page.accessibility_node_payload_for_live_handle(handle).expect("named AX node");
        page.vm_mut().eval("document.getElementById('label').textContent = ''")?;
        let after = page.accessibility_node_payload_for_live_handle(handle).expect("unnamed AX node");
        assert_eq!(after["nodeId"], before["nodeId"]);
        assert_eq!(after["role"]["value"], "generic");
        assert!(after["name"].is_null());
        assert!(nodes.iter().any(|node| node["role"]["value"] == "StaticText"
            && node["name"]["value"] == "Visible body text" && node["ignored"] == false));
        Ok::<_, anyhow::Error>(())
    }).await.expect("implicit section roles follow the live accessible name");
}

#[tokio::test(flavor = "current_thread")]
async fn presentational_role_conflicts_limit_contenteditable_to_html() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/editing.html")?,
        );
        page.vm_mut().eval(r#"
            document.body.innerHTML = `
                <h2 id="editor" contenteditable="plaintext-only" role="none">Editor</h2>
                <svg>
                    <g id="editable" contenteditable="true" role="none"><text>SVG editor</text></g>
                    <g id="plain" role="presentation"><text>SVG group</text></g>
                    <g id="tabindex" contenteditable="true" role="presentation" tabindex="0"><text>Focusable</text></g>
                    <a id="link" contenteditable="true" role="none" href="/go"><text>Go</text></a>
                    <foreignObject contenteditable="true" width="100" height="100">
                        <h2 id="embedded" contenteditable="true" role="none">Embedded HTML editor</h2>
                    </foreignObject>
                </svg>`;
        "#)?;
        for (id, role) in [
            ("editor", "heading"),
            ("editable", "none"),
            ("plain", "none"),
            ("tabindex", "generic"),
            ("link", "link"),
            ("embedded", "heading"),
        ] {
            let handle = page.vm().document_runtime.dom_host().element_handle_by_id(id)
                .expect("fixture element");
            let node = page.accessibility_node_payload_for_live_handle(handle).expect("local AX node");
            assert_eq!(node["role"]["value"], role, "{id}");
            if id == "link" {
                assert!(node["properties"].as_array().unwrap().iter().any(|property|
                    property["name"] == "url" && property["value"]["value"] == "https://example.test/go"
                ));
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("only HTML contenteditable affects presentational role conflicts");
}

#[tokio::test(flavor = "current_thread")]
async fn local_accessibility_requests_only_allocate_backend_ids_used_by_the_payload() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/ax-lazy.html")?,
        );
        page.vm_mut().eval(r#"
          document.body.innerHTML = '<span id=before></span><div id=local><button id=target aria-label=Action><span>Contents</span></button></div><div id=unrelated></div><span id=after></span>';
          unrelated.innerHTML = '<div><button>Unrelated</button></div>'.repeat(512);
        "#)?;
        let handle = |id| {
            page.vm().document_runtime.dom_host()
                .element_handle_by_id(id).expect("fixture element")
        };
        let (before, target, after) = (handle("before"), handle("target"), handle("after"));
        let before_id = page.renderer_backend_node_id_for_live_handle(before)
            .expect("before marker");
        let payload = page.accessibility_node_payload_for_live_handle(target)
            .expect("local AX node");
        assert_eq!(payload["name"]["value"], "Action");
        let after_id = page.renderer_backend_node_id_for_live_handle(after)
            .expect("after marker");
        let mut referenced = std::collections::HashSet::from([
            payload["backendDOMNodeId"].as_u64().expect("backend id") as u32,
        ]);
        referenced.insert(
            payload["parentId"].as_str().expect("parent ref")
                .strip_prefix("AX-").expect("AX ref").parse::<u32>()?,
        );
        for child in payload["childIds"].as_array().expect("direct child refs") {
            referenced.insert(
                child.as_str().expect("child ref")
                    .strip_prefix("AX-").expect("AX ref").parse::<u32>()?,
            );
        }
        assert_eq!(
            (before_id + 1..after_id).collect::<std::collections::HashSet<_>>(),
            referenced.into_iter().filter(|id| *id > before_id).collect(),
            "AX must not allocate ids for the unrelated Document subtree",
        );
        assert_eq!(
            page.accessibility_node_payload_for_live_handle(target)
                .expect("repeated AX node"),
            payload,
            "request-local caching must preserve stable refs",
        );
        Ok::<_, anyhow::Error>(())
    }).await.expect("local AX requests must allocate refs on demand");
}

#[tokio::test(flavor = "current_thread")]
async fn accessibility_style_reads_never_publish_cold_or_warm_layout() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.test/ax-style.html")?,
        );
        page.vm_mut().set_layout_policy(crate::real_layout_test_policy());
        page.vm_mut().eval(r#"
          document.body.innerHTML = '<button id=target>Live action</button><div id=host role=button></div>';
          const root = host.attachShadow({mode:'closed'});
          root.innerHTML = '<span>Shadow name</span>';
        "#)?;
        let cache = page.vm().layout_snapshot_cache_observability_for_test();
        assert!(cache.3.is_none());
        let passes = page.vm().layout_pass_observability_for_test().1;
        let nodes = page.accessibility_tree_payloads_for_document(None).expect("cold AX tree");
        assert!(nodes.iter().any(|node| node["name"]["value"] == "Live action"));
        assert!(nodes.iter().any(|node| node["name"]["value"] == "Shadow name"));
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(page.vm().layout_snapshot_cache_observability_for_test(), cache);

        page.vm_mut().publish_layout_for_test()?;
        let passes = page.vm().layout_pass_observability_for_test().1;
        let cache = page.vm().layout_snapshot_cache_observability_for_test();
        page.vm_mut().eval("target.style.display='none';root.querySelector('span').style.display='none'")?;
        let nodes = page.accessibility_tree_payloads_for_document(None).expect("warm AX tree");
        assert!(!nodes.iter().any(|node| node["name"]["value"] == "Live action" || node["name"]["value"] == "Shadow name"));
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        let after = page.vm().layout_snapshot_cache_observability_for_test();
        assert_eq!((after.2, after.3), (cache.2, cache.3), "AX must preserve the published geometry after a style mutation");
        Ok::<_, anyhow::Error>(())
    }).await.expect("AX style observation must stay independent of layout");
}
