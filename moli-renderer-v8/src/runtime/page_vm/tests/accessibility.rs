use super::*;

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
