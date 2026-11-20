use super::*;
use crate::{
    RendererDocumentFrontendNodeIdsResolution, RendererDomMutationEvent,
    RendererDomSearchResultsResolution,
};
use moli_page_types::DocumentNodeSnapshot;

fn mutation_interest_fixture(observer: bool) -> PageVm {
    let mut page = test_page_vm();
    page.vm_mut()
        .eval(
            "document.body.innerHTML = '<main id=probe>before</main>'; \
             globalThis.round = 0; \
             globalThis.mutateProbe = () => { \
               const probe = document.getElementById('probe'); \
               const value = String(++globalThis.round); \
               probe.setAttribute('data-round', value); \
               probe.firstChild.data = value; \
               probe.appendChild(document.createElement('i')); \
               return globalThis.observer ? observer.takeRecords().length : 0; \
             };",
        )
        .unwrap();
    if observer {
        page.vm_mut()
            .eval(
                "globalThis.observer = new MutationObserver(() => {}); \
                 observer.observe(document.getElementById('probe'), \
                   {attributes:true, characterData:true, childList:true, subtree:true});",
            )
            .unwrap();
    }
    page
}

fn mutate_probe_and_count_facts(page: &mut PageVm, observer: bool) -> [usize; 3] {
    let records = page.vm_mut().eval("mutateProbe()").unwrap();
    assert_eq!(records, if observer { "3" } else { "0" });
    let mut counts = [0; 3];
    for fact in page
        .vm_mut()
        .document_runtime
        .take_pending_devtools_dom_mutations()
    {
        counts[match fact {
            crate::document_runtime::DevToolsDomMutationFact::Attribute { .. } => 0,
            crate::document_runtime::DevToolsDomMutationFact::CharacterData { .. } => 1,
            crate::document_runtime::DevToolsDomMutationFact::ChildList(_) => 2,
        }] += 1;
    }
    counts
}

fn allocate_internal_probe_snapshot(page: &mut PageVm) -> u32 {
    let probe = page.vm().element_handle_by_id_for_test("probe").unwrap();
    let object = page
        .resolve_runtime_object_for_live_handle(Some("internal"), probe, None, None)
        .unwrap()
        .unwrap();
    let snapshot = page
        .document_node_snapshot_for_object_id(
            Some("internal"),
            object.as_protocol_value()["objectId"].as_str().unwrap(),
            1,
            false,
            true,
        )
        .unwrap()
        .unwrap();
    assert!(snapshot.snapshot.frontend_node_id.is_some_and(|id| id != 0));
    let backend = snapshot.snapshot.backend_node_id.unwrap();
    assert!(!page.document_node_is_published(Some("internal"), backend));
    backend
}

#[test]
fn internal_snapshots_do_not_collect_inspector_mutation_facts() {
    let mut observed = Vec::new();
    for bidi in [false, true] {
        for observer in [false, true] {
            let mut page = mutation_interest_fixture(observer);
            let backend = allocate_internal_probe_snapshot(&mut page);
            if bidi {
                page.register_document_bidi_node_binding(
                    Some("internal"),
                    "bidi-node".to_owned(),
                    backend,
                );
            }
            observed.push((
                bidi,
                observer,
                mutate_probe_and_count_facts(&mut page, observer),
            ));
        }
    }
    assert!(
        observed.iter().all(|(_, _, counts)| *counts == [0; 3]),
        "unpublished automation bindings captured Inspector facts: {observed:?}"
    );
}

#[test]
fn mutation_recording_follows_publication_disable_detach_and_document_open() {
    let mut page = mutation_interest_fixture(false);
    allocate_internal_probe_snapshot(&mut page);
    for session in ["cdp-a", "cdp-b"] {
        page.document_node_snapshot_for_document(Some(session), -1, false)
            .unwrap();
        assert_eq!(mutate_probe_and_count_facts(&mut page, false), [1; 3]);
    }
    page.vm_mut()
        .detach_runtime_inspector_session(Some("cdp-b"));
    assert_eq!(
        mutate_probe_and_count_facts(&mut page, false),
        [1; 3],
        "detaching one session must preserve the other's mutation stream"
    );
    page.discard_document_frontend_bindings(Some("cdp-a"));
    assert_eq!(
        mutate_probe_and_count_facts(&mut page, false),
        [0; 3],
        "an internal binding must not keep capture enabled after DOM.disable"
    );
    page.document_node_snapshot_for_document(Some("cdp-a"), -1, false)
        .unwrap();
    page.vm_mut().eval(
        "document.open(); document.write('<html><body><main id=probe>new</main></body></html>'); document.close()",
    ).unwrap();
    page.vm_mut()
        .document_runtime
        .take_pending_devtools_dom_mutations();
    assert_eq!(
        mutate_probe_and_count_facts(&mut page, false),
        [0; 3],
        "a replacement Document must wait for a fresh publication"
    );
    page.document_node_snapshot_for_document(Some("cdp-a"), -1, false)
        .unwrap();
    assert_eq!(mutate_probe_and_count_facts(&mut page, false), [1; 3]);
    page.vm_mut()
        .detach_runtime_inspector_session(Some("cdp-a"));
    assert_eq!(
        mutate_probe_and_count_facts(&mut page, false),
        [0; 3],
        "detaching the last published session must stop capture"
    );
}

fn inspector_descendants(snapshot: &DocumentNodeSnapshot) -> Vec<&DocumentNodeSnapshot> {
    let mut pending = vec![snapshot];
    let mut nodes = Vec::new();
    while let Some(node) = pending.pop() {
        pending.extend(&node.children);
        pending.extend(&node.shadow_roots);
        pending.extend(&node.pseudo_elements);
        if let Some(associated) = node.associated.as_deref() {
            pending.push(associated.node());
        }
        nodes.push(node);
    }
    nodes
}

fn snapshot_marker_hosts(snapshot: &DocumentNodeSnapshot) -> std::collections::BTreeSet<String> {
    inspector_descendants(snapshot)
        .into_iter()
        .filter(|node| {
            node.pseudo_elements
                .iter()
                .any(|pseudo| pseudo.pseudo_type.as_deref() == Some("marker"))
        })
        .filter_map(|node| node.attributes.iter().find(|attr| attr.local_name == "id"))
        .map(|attr| attr.value.clone())
        .collect()
}

fn snapshot_marker_id(snapshot: &DocumentNodeSnapshot) -> Option<u32> {
    snapshot
        .pseudo_elements
        .iter()
        .find(|pseudo| pseudo.pseudo_type.as_deref() == Some("marker"))?
        .backend_node_id
}

fn retain_inspector_object(page: &mut PageVm, expression: &str) -> anyhow::Result<String> {
    let request = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {"expression": expression},
    });
    let messages = page
        .vm_mut()
        .dispatch_inspector_protocol_message(&request.to_string())?;
    Ok(messages
        .iter()
        .find(|message| message["id"] == 1)
        .and_then(|message| message["result"]["result"]["objectId"].as_str())
        .expect("Inspector must retain the requested node")
        .to_owned())
}

#[tokio::test(flavor = "current_thread")]
async fn document_snapshot_batches_marker_styles_without_layout() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/inspector-markers")?,
            );
        page.vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        page.vm_mut().eval(
            r#"
            document.body.innerHTML = `
                <li id="native">native marker</li>
                <div id="custom" style="display:list-item;list-style:none">custom marker</div>
                <li id="suppressed" style="list-style:none">no marker</li>
                <div id="hidden" style="display:none">
                    <div id="revealed" style="display:list-item">hidden marker</div>
                </div>
                <div id="host">
                    <div id="slotted" style="display:list-item" slot="shown">slotted</div>
                    <div id="unslotted" style="display:list-item">unslotted</div>
                </div>
                <iframe id="frame" style="width:100px"></iframe>`;
            const sheet = document.createElement('style');
            sheet.textContent = '#custom::marker { content: "custom" }';
            document.head.append(sheet);
            document.getElementById('host').attachShadow({mode:'open'})
                .innerHTML = '<slot name="shown"></slot>';
        "#,
        )?;
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame")?;
        page.vm_mut().eval(
            r#"
            document.getElementById('frame').contentDocument.body.innerHTML =
                '<style>@media (max-width:150px) { #child { display:list-item } }</style>' +
                '<div id="child">frame marker</div>';
        "#,
        )?;

        let marker_hosts = |page: &mut PageVm| {
            let snapshot = page
                .document_node_snapshot_for_document(Some("markers"), -1, true)
                .expect("complete Inspector document");
            snapshot_marker_hosts(&snapshot.snapshot)
        };
        let passes = page.vm().layout_pass_observability_for_test().1;
        let expected = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect();
        assert_eq!(
            marker_hosts(&mut page),
            expected(&["native", "custom", "slotted", "child"])
        );

        page.vm_mut().eval(
            r#"
            document.getElementById('custom').style.display = 'block';
            document.getElementById('hidden').style.display = 'block';
        "#,
        )?;
        assert_eq!(
            marker_hosts(&mut page),
            expected(&["native", "revealed", "slotted", "child"])
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("marker snapshots must share style preparation and never force layout");
}

#[tokio::test(flavor = "current_thread")]
async fn document_snapshot_omits_markers_from_detached_iframe_shadow_tree() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/detached-inspector-marker")?,
            );
        page.vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        page.vm_mut()
            .eval("document.body.innerHTML = '<iframe id=\"frame\"></iframe>'")?;
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame")?;
        page.vm_mut().eval(
            r#"
            const frame = document.getElementById('frame');
            const child = frame.contentDocument;
            globalThis.markerHost = child.body.appendChild(child.createElement('div'));
            const shadow = markerHost.attachShadow({mode: 'open'});
            shadow.innerHTML = '<li style="display:list-item">item</li>';
            globalThis.markerItem = shadow.firstElementChild;
            "#,
        )?;

        let object_id = retain_inspector_object(&mut page, "markerItem")?;
        let describe = |page: &mut PageVm| {
            page.document_node_snapshot_for_object_id(None, &object_id, 1, false, true)
                .expect("Inspector object lookup must succeed")
                .expect("Inspector must still resolve the list item")
                .snapshot
        };

        let passes = page.vm().layout_pass_observability_for_test().1;
        let connected = describe(&mut page);
        assert_eq!(connected.pseudo_elements.len(), 1);
        let marker = &connected.pseudo_elements[0];
        assert_eq!(marker.pseudo_type.as_deref(), Some("marker"));
        let marker_id = marker
            .backend_node_id
            .expect("marker must have a backend id");

        assert_eq!(
            page.vm_mut()
                .eval("markerHost.remove(); frame.isConnected && !markerItem.isConnected")?,
            "true"
        );
        let detached = describe(&mut page);
        assert_eq!(detached.backend_node_id, connected.backend_node_id);
        assert!(detached.pseudo_elements.is_empty());
        assert!(
            page.document_node_snapshot_for_backend_node_id(marker_id, 0, false)?
                .is_none()
        );

        page.vm_mut()
            .eval("frame.contentDocument.body.append(markerHost)")?;
        let reconnected = describe(&mut page);
        assert_eq!(reconnected.backend_node_id, connected.backend_node_id);
        assert_eq!(reconnected.pseudo_elements.len(), 1);
        assert_eq!(
            reconnected.pseudo_elements[0].pseudo_type.as_deref(),
            Some("marker")
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("marker snapshots must respect Shadow DOM attachment without forcing layout");
}

#[tokio::test(flavor = "current_thread")]
async fn document_snapshot_and_layout_agree_on_marker_generation() {
    use moli_layout::{
        LayoutBoxKind, LayoutFlushReason, LayoutPassRequest, PaintCaptureRequest, PaintFragment,
    };

    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/marker-generation")?,
            );
        page.vm_mut().set_layout_policy(crate::real_layout_test_policy());
        page.vm_mut().eval(r#"
            document.body.innerHTML = `
                <style>
                    #empty::marker { content: ""; }
                    #explicit::marker { content: "EXPLICIT"; }
                    #no-content::marker { content: none; color: rgb(17, 34, 51); }
                    #block::marker { content: "ignored"; }
                </style>
                <ol style="list-style-type:decimal; list-style-position:outside">
                    <li id="native">first</li>
                    <li id="suppressed" style="list-style:none">second</li>
                    <li id="following">third</li>
                </ol>
                <li id="empty" style="list-style:none">empty</li>
                <li id="explicit" style="list-style:none">explicit</li>
                <li id="no-content">no content</li>
                <li id="empty-type" style='list-style-type:""'>empty type</li>
                <li id="image" style="list-style-type:none;list-style-image:url(marker.png)">image</li>
                <li id="hidden" style="visibility:hidden">hidden</li>
                <li id="transparent" style="opacity:0">transparent</li>
                <div id="css-item" style="display:list-item">CSS list item</div>
                <div id="block">block</div>
                <noscript id="noscript" style="display:list-item">noscript</noscript>
                <canvas id="canvas" style="display:list-item">
                    <span id="canvas-child" style="display:list-item">fallback</span>
                </canvas>`;
            "#)?;

        let expected = [
            "native", "following", "empty", "explicit", "empty-type", "image", "hidden",
            "transparent", "css-item",
        ].into_iter().map(str::to_owned).collect::<std::collections::BTreeSet<_>>();
        let passes = page.vm().layout_pass_observability_for_test().1;
        crate::style_engine::take_style_resolution_context_count_for_test();
        for contexts in [1, 0] {
            let snapshot = page.document_node_snapshot_for_document(None, -1, true).unwrap();
            assert_eq!(snapshot_marker_hosts(&snapshot.snapshot), expected);
            assert_eq!(crate::style_engine::take_style_resolution_context_count_for_test(), contexts);
        }
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(page.vm_mut().eval(
            "getComputedStyle(document.getElementById('no-content'), '::marker').color"
        )?, "rgb(17, 34, 51)");

        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let (root, viewport) = {
            let host = host.borrow();
            let document = host.document_handle();
            let mut styles = crate::native_bridge::element::StyleObservation::new(&host);
            styles.with_resolution_pass(document, |pass| {
                use crate::style_engine::{MarkerStyleDecision, marker_style_decision};

                let native = host.dom_host().element_handle_by_id("native").unwrap();
                let suppressed = host.dom_host().element_handle_by_id("suppressed").unwrap();
                let native_style = pass.primary_style(native).unwrap();
                let suppressed_style = pass.primary_style(suppressed).unwrap();
                let native_marker = pass.pseudo_style(native, "marker").unwrap();
                let suppressed_marker = pass.pseudo_style(suppressed, "marker").unwrap();
                assert_ne!(native_marker.get_list().list_style_type, suppressed_marker.get_list().list_style_type);
                assert_eq!(
                    marker_style_decision(&native_style, &suppressed_marker),
                    MarkerStyleDecision::DefaultListStyle,
                );
                assert_eq!(
                    marker_style_decision(&suppressed_style, &native_marker),
                    MarkerStyleDecision::Suppressed,
                );
            });
            let root = host.dom_host().document_element_handle().unwrap();
            let boxes = crate::layout_renderer::build_normalized_native_box_tree_for_test(&host, root)?;
            let mut pending = vec![&boxes.root];
            let mut marker_hosts = std::collections::BTreeSet::new();
            while let Some(node) = pending.pop() {
                if node.role == LayoutBoxKind::PseudoMarker {
                    let (_, id) = node.owner.as_deref().unwrap().split_once('#').unwrap();
                    marker_hosts.insert(id.to_owned());
                }
                pending.extend(&node.children);
            }
            assert_eq!(marker_hosts, expected);
            (root, host.layout_viewport_for_document(document))
        };

        let pass = page.vm().build_layout_pass_for_subtree_for_test(
            root,
            LayoutPassRequest::with_capture(
                viewport,
                LayoutFlushReason::Print,
                PaintCaptureRequest::print_document(),
            ),
        )?;
        assert!(pass.paint_snapshot().unwrap().fragments.iter().any(|fragment| {
            matches!(fragment, PaintFragment::GlyphRun(run) if run.text.as_ref().is_some_and(|text| text.text.trim() == "3."))
        }), "the list item with a suppressed marker must still advance the counter");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("DevTools and layout must use the same marker generation semantics");
}

#[tokio::test(flavor = "current_thread")]
async fn document_snapshot_marker_ids_follow_slot_and_iframe_participation() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/marker-participation")?,
            );
        page.vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        page.vm_mut().eval(
            r#"
            document.body.innerHTML = `
                <div id="slotHost"></div>
                <div id="frameHost"><iframe id="frame" style="width:100px"></iframe></div>`;
            globalThis.slotHost = document.getElementById('slotHost');
            const shadow = slotHost.attachShadow({mode:'open'});
            shadow.innerHTML = '<slot><div><li id="fallbackItem">fallback</li></div></slot>';
            globalThis.fallbackItem = shadow.getElementById('fallbackItem');
            globalThis.assignedNode = document.createElement('span');
            globalThis.frame = document.getElementById('frame');
            globalThis.frameHost = document.getElementById('frameHost');
        "#,
        )?;
        materialize_child_realm_through_page_turn_for_test(&mut page, "frame")?;
        page.vm_mut().eval(
            r#"
            frame.contentDocument.body.innerHTML =
                '<style>@media (max-width:150px) { #frameItem { display:list-item } }</style>' +
                '<div id="frameItem">iframe item</div>';
            globalThis.frameItem = frame.contentDocument.getElementById('frameItem');
        "#,
        )?;
        let names = ["fallbackItem", "frameItem"];
        let objects = [
            retain_inspector_object(&mut page, names[0])?,
            retain_inspector_object(&mut page, names[1])?,
        ];
        let passes = page.vm().layout_pass_observability_for_test().1;
        let mut marker_ids = [0; 2];
        let mut owner_ids = [None; 2];
        for (mutation, visible) in [
            ("void 0", [true, true]),
            ("slotHost.append(assignedNode)", [false, true]),
            ("assignedNode.remove()", [true, true]),
            ("frameHost.style.display = 'none'", [true, false]),
            ("frameHost.style.display = ''", [true, true]),
            ("frameItem.remove()", [true, false]),
            ("frame.contentDocument.body.append(frameItem)", [true, true]),
            ("frame.style.width = '250px'", [true, false]),
            ("frame.style.width = '100px'", [true, true]),
        ] {
            page.vm_mut().eval(mutation)?;
            let snapshot = page
                .document_node_snapshot_for_document(None, -1, true)
                .unwrap();
            let expected = names
                .into_iter()
                .zip(visible)
                .filter(|&(_, show)| show)
                .map(|(name, _)| name.to_owned())
                .collect();
            assert_eq!(
                snapshot_marker_hosts(&snapshot.snapshot),
                expected,
                "{mutation}"
            );
            for (index, object_id) in objects.iter().enumerate() {
                let node = page
                    .document_node_snapshot_for_object_id(None, object_id, 1, false, false)?
                    .expect("retained object ID still resolves")
                    .snapshot;
                let owner_id = node.backend_node_id.expect("list item backend ID");
                assert_eq!(*owner_ids[index].get_or_insert(owner_id), owner_id);
                let marker = snapshot_marker_id(&node);
                assert_eq!(
                    marker.is_some(),
                    visible[index],
                    "{mutation}, {}",
                    names[index]
                );
                if let Some(marker) = marker {
                    marker_ids[index] = marker;
                }
                assert_eq!(
                    page.document_node_snapshot_for_backend_node_id(marker_ids[index], 0, false)?
                        .is_some(),
                    visible[index],
                    "{mutation}, backend ID for {}",
                    names[index],
                );
            }
            let RendererDocumentFrontendNodeIdsResolution::Found(frontend_ids) =
                page.document_frontend_node_ids_for_backend_node_ids(None, &marker_ids)
            else {
                panic!("full snapshot must publish the document");
            };
            assert_eq!(
                frontend_ids.iter().map(Option::is_some).collect::<Vec<_>>(),
                visible,
                "{mutation}"
            );
        }
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("marker projection and ID queries must follow slot and frame participation");
}

fn serialized_inspector_ids(snapshot: &DocumentNodeSnapshot) -> std::collections::HashSet<u32> {
    let wire = moli_protocol_cdp::node_snapshot_to_cdp(snapshot, None, None).unwrap();
    let mut pending = vec![&wire];
    let mut ids = std::collections::HashSet::new();
    while let Some(value) = pending.pop() {
        match value {
            serde_json::Value::Object(fields) => {
                if let Some(id) = fields.get("nodeId").and_then(serde_json::Value::as_u64) {
                    ids.insert(u32::try_from(id).unwrap());
                }
                pending.extend(fields.values());
            }
            serde_json::Value::Array(values) => pending.extend(values),
            _ => {}
        }
    }
    ids
}

#[test]
fn child_list_resync_keeps_publication_consistent_beyond_the_wire_depth_limit() {
    run_page_vm_large_stack_async_test("deep-dom-publication", || async {
        for levels in [1000, 1001, 1002, 1100] {
            let mut page = test_page_vm();
            page.vm_mut()
                .eval(&format!(
                    r#"
            document.body.innerHTML = '<main id=parent><i id=previous></i><i id=anchor></i></main>';
            let cursor = document.getElementById('parent');
            for (let i = 0; i < {levels}; ++i) {{
                const child = document.createElement('div');
                child.id = 'level-' + i;
                cursor.appendChild(child);
                cursor = child;
                if (i === 999) {{
                    child.attachShadow({{mode:'closed'}}).innerHTML = '<b>boundary</b>';
                }}
            }}
        "#
                ))
                .unwrap();
            let document = page
                .document_node_snapshot_for_document(Some("session"), 4, false)
                .unwrap();
            let mut visible = serialized_inspector_ids(&document.snapshot);
            // Expand in separate bounded requests, just as a client can walk past
            // the per-message wire limit without ever requesting an oversized tree.
            for index in (0..levels).step_by(200) {
                let handle = page
                    .vm()
                    .element_handle_by_id_for_test(&format!("level-{index}"))
                    .unwrap();
                let backend = page
                    .renderer_backend_node_id_for_live_handle(handle)
                    .unwrap();
                let frontend = page
                    .existing_document_frontend_node_id(Some("session"), backend)
                    .unwrap();
                assert!(visible.contains(&frontend));
                let expanded = page
                    .document_child_node_snapshot_events_for_node(
                        Some("session"),
                        false,
                        crate::RendererDomNodeReference::FrontendNodeId {
                            inspector_session_id: Some("session".to_owned()),
                            frontend_node_id: frontend,
                        },
                        199,
                        false,
                    )
                    .unwrap();
                for event in expanded.events {
                    assert!(visible.contains(&event.parent_frontend_node_id));
                    for node in &event.snapshots {
                        visible.extend(serialized_inspector_ids(node));
                    }
                }
            }
            // Put an expanded associated root at the last wire level as well:
            // reserving its shallow root must not drop its separate child list.
            let boundary_host = page
                .vm()
                .element_handle_by_id_for_test("level-999")
                .unwrap();
            let boundary_root = page
                .vm()
                .document_runtime
                .dom_host()
                .shadow_root_handle(boundary_host)
                .unwrap();
            let boundary_backend = page
                .renderer_backend_node_id_for_live_handle(boundary_root)
                .unwrap();
            let boundary_frontend = page
                .existing_document_frontend_node_id(Some("session"), boundary_backend)
                .unwrap();
            assert!(visible.contains(&boundary_frontend));
            let expanded = page
                .document_child_node_snapshot_events_for_live_handles(
                    Some("session"),
                    &[boundary_root],
                    0,
                    false,
                )
                .unwrap();
            let mut associated_before = vec![(boundary_backend, boundary_frontend)];
            for event in expanded.events {
                assert!(visible.contains(&event.parent_frontend_node_id));
                for node in &event.snapshots {
                    visible.extend(serialized_inspector_ids(node));
                    associated_before.extend(inspector_descendants(node).into_iter().map(|node| {
                        (
                            node.backend_node_id.unwrap(),
                            node.frontend_node_id.unwrap(),
                        )
                    }));
                }
            }
            let mut before = (0..levels)
                .map(|index| {
                    let handle = page
                        .vm()
                        .element_handle_by_id_for_test(&format!("level-{index}"))
                        .unwrap();
                    let backend = page
                        .renderer_backend_node_id_for_live_handle(handle)
                        .unwrap();
                    let frontend = page
                        .existing_document_frontend_node_id(Some("session"), backend)
                        .unwrap();
                    assert!(visible.contains(&frontend));
                    (backend, frontend)
                })
                .collect::<Vec<_>>();
            before.extend(associated_before);
            let parent = page.vm().element_handle_by_id_for_test("parent").unwrap();
            let parent_backend = page
                .renderer_backend_node_id_for_live_handle(parent)
                .unwrap();
            let parent_frontend = page
                .existing_document_frontend_node_id(Some("session"), parent_backend)
                .unwrap();
            let previous = page.vm().element_handle_by_id_for_test("previous").unwrap();
            let previous_backend = page
                .renderer_backend_node_id_for_live_handle(previous)
                .unwrap();
            page.take_pending_dom_mutation_event_batches();
            page.dom_agent_state
                .remove_frontend_bindings_for_backend_node_ids(
                    Some("session"),
                    page.current_dom_agent_document_id(),
                    [previous_backend],
                );
            page.vm_mut()
                .eval("document.getElementById('previous').after(document.createElement('span'))")
                .unwrap();
            let batches = page.take_pending_dom_mutation_event_batches();
            let replacements = batches
                .iter()
                .flat_map(|batch| &batch.events)
                .filter_map(|event| match event {
                    RendererDomMutationEvent::SetChildNodes {
                        parent_node_id,
                        nodes,
                    } => Some((*parent_node_id, nodes)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                replacements.first().map(|(id, _)| *id),
                Some(parent_frontend)
            );
            // The ancestor replacement retires its old subtree in the client.
            visible = std::collections::HashSet::from([parent_frontend]);
            for (parent, nodes) in replacements {
                assert!(
                    visible.contains(&parent),
                    "notification references an unknown parent"
                );
                for node in nodes {
                    visible.extend(serialized_inspector_ids(node));
                }
            }
            for (backend, frontend) in before {
                assert!(page.document_node_is_published(Some("session"), backend));
                assert!(
                    visible.contains(&frontend),
                    "{levels}-level resync retained unpublished wire ID {frontend}"
                );
            }
        }
    });
}

#[tokio::test(flavor = "current_thread")]
async fn child_list_resync_preserves_published_inspector_subtrees() {
    run_page_vm_async_test(async move {
        for include_whitespace in [false, true] {
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
            let (mut page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse("https://example.com/inspector-resync")?,
                );
            page_vm.vm_mut().eval(r#"
                document.body.innerHTML = `<main id="parent">
                    <i id="previous"></i><i id="anchor"></i>
                    <iframe id="frame"></iframe>
                    <input id="control" type="search" value="needle">
                    <li id="marker">marker</li>
                    <div id="host"><em id="retained">light</em></div>
                    <template id="template"><article><span>inside</span>
                        <template><b>nested</b></template></article></template>
                </main>`;
                document.getElementById('host').attachShadow({mode:'closed'})
                    .innerHTML = '<section><a id="shadow-hit">shadow</a></section>';
            "#)?;
            materialize_child_realm_through_page_turn_for_test(&mut page_vm, "frame")?;
            page_vm.vm_mut().eval(
                "document.getElementById('frame').contentDocument.body.innerHTML = '<main><a id=frame-hit>frame</a></main>'",
            )?;
            page_vm.configure_document_dom_agent_session(Some("session"), include_whitespace);
            let document = page_vm
                .document_node_snapshot_for_document(Some("session"), -1, true)
                .expect("complete Inspector document");
            let parent = page_vm.vm().element_handle_by_id_for_test("parent").unwrap();
            let previous = page_vm.vm().element_handle_by_id_for_test("previous").unwrap();
            let initial = inspector_descendants(&document.snapshot);
            let parent_snapshot = initial.iter().find(|node| node.node_id == parent).unwrap();
            let mut before = inspector_descendants(parent_snapshot).into_iter().cloned().collect::<Vec<_>>();
            assert!(before.iter().any(|node| node.associated.is_some()), "iframe document must be published");
            assert!(before.iter().any(|node| node.shadow_root_type.as_deref() == Some("user-agent")), "generated UA root must be published");
            assert!(before.iter().any(|node| !node.pseudo_elements.is_empty()), "generated marker must be published");
            let mut fragments = before.iter().filter_map(|node| node.template_content().map(|content| content.node_id)).collect::<Vec<_>>();
            while let Some(fragment) = fragments.pop() {
                let expanded = page_vm.document_child_node_snapshot_events_for_live_handles(
                    Some("session"), &[fragment], -1, true,
                ).expect("expand template content");
                for node in expanded.events.iter().flat_map(|event| &event.snapshots).flat_map(inspector_descendants) {
                    if let Some(content) = node.template_content() {
                        fragments.push(content.node_id);
                    }
                    before.push(node.clone());
                }
            }
            let parent_id = parent_snapshot.frontend_node_id.unwrap();
            let document_id = page_vm.current_dom_agent_document_id();
            let previous_backend = page_vm.renderer_backend_node_id_for_live_handle(previous).unwrap();
            page_vm.take_pending_dom_mutation_event_batches();
            // Exercise the fallback used when an incremental insertion cannot
            // name its previous sibling, rather than calling the helper directly.
            page_vm.dom_agent_state.remove_frontend_bindings_for_backend_node_ids(
                Some("session"), document_id, [previous_backend],
            );
            page_vm.vm_mut().eval(
                "document.getElementById('previous').after(document.createElement('span'))",
            )?;
            let batches = page_vm.take_pending_dom_mutation_event_batches();
            let replacement_events = batches.iter()
                .filter(|batch| batch.session.wire_session_id() == Some("session"))
                .flat_map(|batch| &batch.events)
                .filter_map(|event| match event {
                    RendererDomMutationEvent::SetChildNodes { parent_node_id, nodes } => Some((*parent_node_id, nodes)),
                    _ => None,
                }).collect::<Vec<_>>();
            assert_eq!(replacement_events.first().map(|(id, _)| *id), Some(parent_id),
                "missing previous sibling must resynchronize the parent first");
            assert!(replacement_events.len() >= 3, "expanded template fragments need separate notifications");
            let mut visible = std::collections::HashSet::from([parent_id]);
            for (parent, nodes) in replacement_events {
                assert!(visible.contains(&parent), "fragment notification preceded its owner");
                for node in nodes {
                    visible.extend(serialized_inspector_ids(node));
                }
            }
            assert!(!visible.contains(&0));
            // Replacing the parent's children retires all old descendants in
            // the client mirror. Any ID the renderer still publishes must be
            // present in the replacement, including associated trees.
            for old in before.into_iter().filter(|node| node.node_id != parent) {
                let backend = old.backend_node_id.unwrap();
                let frontend = old.frontend_node_id.unwrap();
                if page_vm.document_node_is_published(Some("session"), backend)
                    && page_vm.existing_document_frontend_node_id(Some("session"), backend) == Some(frontend)
                {
                    assert!(visible.contains(&frontend),
                        "resync lost published {} node {frontend} (whitespace={include_whitespace})", old.node_name);
                }
            }
            for (query, ua) in [("#frame-hit", false), ("#shadow-hit", false), ("#editing-view-port", true)] {
                let search = page_vm.document_perform_search(Some("session"), query, ua);
                assert_eq!(search.result_count, 1, "{query}");
                let (resolution, _) = page_vm.document_search_results_with_child_node_snapshot_events(
                    Some("session"), &search.search_id, 0, 1,
                );
                let RendererDomSearchResultsResolution::Found(results) = resolution else {
                    panic!("missing retained search result for {query}");
                };
                assert!(visible.contains(&results[0].frontend_node_id), "search returned retired ID for {query}");
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("Inspector resync fixture should run");
}
