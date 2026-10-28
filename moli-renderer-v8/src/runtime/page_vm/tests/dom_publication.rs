use super::*;
use crate::{RendererDomMutationEvent, RendererDomSearchResultsResolution};
use moli_page_types::DocumentNodeSnapshot;

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
