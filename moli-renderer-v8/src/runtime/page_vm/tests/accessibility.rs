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
            assert_eq!(node["ignored"], role == "none", "{id}");
            if role == "none" {
                assert_eq!(node["ignoredReasons"][0]["name"], "presentationalRole", "{id}");
            }
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
async fn dom_focus_and_ax_use_the_same_native_element_rules() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.test/focus.html")?,
        );
        page.vm_mut().eval(
            r##"
            document.body.innerHTML = `
                <h2 id="invalid" role="none" aria-level="3" tabindex="&#11;0">Kept text</h2>
                <h2 id="valid" role="none" tabindex="&#9;-1">Focusable heading</h2>
                <h2 id="spaced" role="none" contenteditable=" true ">Invalid editing keyword</h2>
                <fieldset disabled>
                    <legend><button id="legend">Legend control</button></legend>
                    <button id="disabled">Disabled control</button>
                    <a id="link" role="none" href="#link">Link in fieldset</a>
                </fieldset>`;
        "##,
        )?;
        for (id, role, focusable, disabled) in [
            ("invalid", "none", false, false),
            ("valid", "heading", true, false),
            ("spaced", "none", false, false),
            ("legend", "button", true, false),
            ("disabled", "button", false, true),
            ("link", "link", true, false),
        ] {
            let state = page.vm_mut().eval(&format!(
                r#"
                (() => {{
                    document.activeElement.blur();
                    const element = document.getElementById('{id}');
                    element.focus();
                    return JSON.stringify([document.activeElement === element,
                        element.matches(':disabled')]);
                }})()
            "#
            ))?;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&state)?,
                serde_json::json!([focusable, disabled]),
                "{id}"
            );
            let dom = page.vm().document_runtime.dom_host();
            let handle = dom.element_handle_by_id(id).expect("fixture element");
            let node = page
                .accessibility_node_payload_for_live_handle(handle)
                .expect("AX node");
            assert_eq!(node["role"]["value"], role, "{id}");
            assert_eq!(node["ignored"], role == "none", "{id}");
            let property_is_true = |name| {
                node["properties"].as_array().is_some_and(|properties| {
                    properties.iter().any(|property| {
                        property["name"] == name && property["value"]["value"] == true
                    })
                })
            };
            assert_eq!(property_is_true("focusable"), focusable, "{id}");
            assert_eq!(property_is_true("disabled"), disabled, "{id}");
        }
        let nodes = page
            .accessibility_tree_payloads_for_document(None)
            .expect("full AX tree");
        assert!(
            nodes
                .iter()
                .any(|node| node["role"]["value"] == "StaticText"
                    && node["name"]["value"] == "Kept text"
                    && node["ignored"] == false)
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DOM focus and AX properties follow native control semantics");
}

#[tokio::test(flavor = "current_thread")]
async fn select_disabledness_agrees_across_selectors_focus_and_ax() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.test/select.html")?,
        );
        page.vm_mut().eval(r#"
            document.body.innerHTML = `
                <select id="locked" disabled>
                    <option id="intrinsic" disabled>Unavailable</option>
                    <optgroup id="group" label="Choices">
                        <option id="first" tabindex="0" value="first">First</option>
                    </optgroup>
                </select>
                <fieldset disabled>
                    <legend><select><option id="legend">Legend choice</option></select></legend>
                    <select><option id="fieldset">Fieldset choice</option></select>
                </fieldset>`;
            document.getElementById('legend').setAttributeNS('urn:test', 'disabled', '');
        "#)?;
        for select_disabled in [true, false] {
            let selected = page.vm_mut().eval(&format!(r#"
                (() => {{
                    const select = document.getElementById('locked');
                    select.disabled = {select_disabled};
                    return JSON.stringify([select.value, select.selectedIndex, select.selectedOptions[0].disabled]);
                }})()
            "#))?;
            assert_eq!(selected, r#"["first",1,false]"#);
            for (id, disabled) in [
                ("intrinsic", true), ("group", select_disabled), ("first", select_disabled),
                ("legend", false), ("fieldset", true),
            ] {
                let state = page.vm_mut().eval(&format!(r#"
                    (() => {{
                        const element = document.getElementById('{id}');
                        element.focus();
                        return JSON.stringify([element.matches(':disabled'), element.matches(':enabled'),
                            document.activeElement === element]);
                    }})()
                "#))?;
                let state: serde_json::Value = serde_json::from_str(&state)?;
                assert_eq!(state[0], disabled, "{id}");
                assert_eq!(state[1], !disabled, "{id}");
                if disabled {
                    assert_eq!(state[2], false, "{id}");
                }

                let handle = page.vm().document_runtime.dom_host().element_handle_by_id(id)
                    .expect("fixture element");
                let node = page.accessibility_node_payload_for_live_handle(handle).expect("AX node");
                assert_eq!(node["ignored"], false, "{id}");
                let properties = node["properties"].as_array().expect("AX properties");
                assert_eq!(properties.iter().any(|property| property["name"] == "disabled"
                    && property["value"]["value"] == true), disabled, "{id}");
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("actually disabled options retain their selection state");
}

#[tokio::test(flavor = "current_thread")]
async fn select_validation_and_ax_ignore_namespaced_display_size() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.test/select-size.html")?,
        );
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<select id="test" required><option value="" selected>Choose</option><option>Valid</option></select>';
            document.getElementById('test').setAttributeNS('urn:test', 'size', '4');
        "#)?;
        let handle = page.vm().document_runtime.dom_host().element_handle_by_id("test")
            .expect("select");
        for (native_size, role, invalid) in [(false, "combobox", true), (true, "listbox", false)] {
            if native_size {
                page.vm_mut().eval("document.getElementById('test').setAttributeNS(null, 'size', '4')")?;
            }
            let result = page.vm_mut().eval(r#"
                (() => {
                    const select = document.getElementById('test');
                    return JSON.stringify([select.matches(':invalid'), select.validity.valueMissing]);
                })()
            "#)?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, serde_json::json!([invalid, invalid]));
            let node = page.accessibility_node_payload_for_live_handle(handle).expect("AX node");
            assert_eq!(node["role"]["value"], role);
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("select display size uses the null namespace");
}

#[tokio::test(flavor = "current_thread")]
async fn ax_link_urls_match_html_area_and_svg_activation() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.test/links.html")?,
        );
        page.vm_mut().eval(r##"
            document.body.innerHTML = `
                <img usemap="#map" width="40" height="40" alt="Map">
                <map name="map"><area id="area" href="#area" shape="rect" coords="0,0,40,40" alt="Area" role="none"></map>
                <svg xmlns:xlink="http://www.w3.org/1999/xlink">
                    <a id="svg" xlink:href="#svg" role="none"><text>SVG</text></a>
                    <a id="foreign" role="link"><text>Foreign</text></a>
                </svg>`;
            document.getElementById('svg').setAttributeNS('urn:test', 'src:href', '#wrong');
            document.getElementById('foreign').setAttributeNS('urn:test', 'src:href', '#wrong');
        "##)?;
        for (id, name, url) in [
            ("area", "Area", Some("https://example.test/links.html#area")),
            ("svg", "SVG", Some("https://example.test/links.html#svg")),
            ("foreign", "Foreign", None),
        ] {
            let handle = page.vm().document_runtime.dom_host().element_handle_by_id(id)
                .expect("fixture element");
            let node = page.accessibility_node_payload_for_live_handle(handle).expect("AX node");
            assert_eq!(node["role"]["value"], "link", "{id}");
            assert_eq!(node["name"]["value"], name, "{id}");
            let properties = node["properties"].as_array().expect("AX link properties");
            let actual_url = properties.iter().find(|property| property["name"] == "url")
                .and_then(|property| property["value"]["value"].as_str());
            assert_eq!(actual_url, url, "{id}");
        }
        assert_eq!(page.vm_mut().eval("document.getElementById('area').click(); location.hash")?, "#area");
        for (id, hash) in [("svg", "#svg"), ("foreign", "#svg")] {
            assert_eq!(page.vm_mut().eval(&format!(r#"
                document.getElementById('{id}').dispatchEvent(new MouseEvent('click', {{bubbles: true}}));
                location.hash
            "#))?, hash, "{id}");
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("AX link destinations agree with click activation");
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
