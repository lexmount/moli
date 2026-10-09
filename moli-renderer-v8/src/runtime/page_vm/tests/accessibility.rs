use super::*;

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
