use super::*;

fn document() -> (DomHost, DomHandle) {
    let mut host = DomHost::from_dom(NativeDom::new_html(
        Url::parse("https://shadow-frame-candidates.test/").unwrap(),
    ));
    host.reset_html_document_shell();
    let body = host.document_body_handle().unwrap();
    (host, body)
}

fn frames(host: &DomHost, root: DomHandle) -> Vec<DomHandle> {
    host.child_browsing_context_host_candidate_handles_in_shadow_including_subtree_order(root)
}

#[test]
fn frame_candidates_follow_physical_shadow_order_after_tree_moves() {
    let (mut host, body) = document();
    let outer = host.create_element("div");
    let shadow = host.attach_shadow_root(outer, "open").unwrap();
    // Creation order differs from both light and shadow-including tree order.
    let light = host.create_element("iframe");
    let last = host.create_element("embed");
    let nested = host.create_element("frame");
    let first = host.create_element("object");
    let nested_host = host.create_element("span");
    let nested_shadow = host.attach_shadow_root(nested_host, "closed").unwrap();
    let outside = host.create_element("iframe");
    assert!(host.append_child(body, outside));
    assert!(host.append_child(shadow, first));
    assert!(host.append_child(shadow, nested_host));
    assert!(host.append_child(nested_shadow, nested));
    assert!(host.append_child(shadow, last));
    assert!(host.append_child(outer, light));
    assert!(host.append_child(body, outer));

    assert_eq!(frames(&host, outer), vec![first, nested, last, light]);
    assert_eq!(frames(&host, shadow), vec![first, nested, last]);
    assert_eq!(frames(&host, light), vec![light]);
    assert_eq!(
        frames(&host, body),
        vec![outside, first, nested, last, light]
    );
    assert_eq!(
        host.child_browsing_context_host_candidate_handles_in_subtree_in_document_order(body),
        vec![outside, light],
        "the light-tree query keeps its existing scope",
    );

    assert!(host.append_child(outer, first));
    assert_eq!(frames(&host, outer), vec![nested, last, light, first]);
    assert!(host.append_child(shadow, nested));
    assert_eq!(frames(&host, outer), vec![last, nested, light, first]);
}

#[test]
fn reconnecting_shadow_trees_restores_compacted_frame_candidates() {
    let (mut host, body) = document();
    let outer = host.create_element("div");
    let shadow = host.attach_shadow_root(outer, "closed").unwrap();
    let frame = host.create_element("iframe");
    assert!(host.append_child(shadow, frame));
    host.compact_child_browsing_context_host_candidates();
    assert!(
        host.child_browsing_context_host_candidate_handles()
            .is_empty()
    );
    assert_eq!(frames(&host, outer), vec![frame]);

    assert!(host.append_child(body, outer));
    assert_eq!(
        host.child_browsing_context_host_candidate_handles(),
        vec![frame]
    );
    assert_eq!(frames(&host, body), vec![frame]);
    assert!(host.remove_child(body, outer));
    host.compact_child_browsing_context_host_candidates();
    assert!(frames(&host, body).is_empty());
    assert_eq!(frames(&host, shadow), vec![frame]);

    assert!(host.append_child(body, outer));
    assert_eq!(
        host.child_browsing_context_host_candidate_handles(),
        vec![frame]
    );
    assert_eq!(frames(&host, body), vec![frame]);
}

#[test]
fn cloned_imported_and_adopted_shadow_frames_keep_candidate_membership() {
    let (mut host, body) = document();
    let source = host.create_element("section");
    let mut init = ShadowRootInit::new("closed");
    init.set_clonable(true);
    let shadow = host.attach_shadow_root_with_init(source, init).unwrap();
    let source_frame = host.create_element("iframe");
    assert!(host.append_child(shadow, source_frame));
    let cloned = host.clone_node(source, true).unwrap();
    let imported = host
        .import_node(host.document_handle(), source, true)
        .unwrap();
    let clone_frame = host
        .first_child(host.shadow_root_handle(cloned).unwrap())
        .unwrap();
    let imported_frame = host
        .first_child(host.shadow_root_handle(imported).unwrap())
        .unwrap();
    host.compact_child_browsing_context_host_candidates();
    assert!(host.append_child(body, cloned));
    assert!(host.append_child(body, imported));
    assert_eq!(frames(&host, body), vec![clone_frame, imported_frame]);

    let secondary = host.create_detached_html_document();
    assert_eq!(host.adopt_node(secondary, cloned), Some(cloned));
    assert!(host.append_child(secondary, cloned));
    host.compact_child_browsing_context_host_candidates();
    assert_eq!(frames(&host, body), vec![imported_frame]);
    assert_eq!(frames(&host, secondary), vec![clone_frame]);
    assert_eq!(host.owner_document_handle(clone_frame), Some(secondary));

    assert!(host.append_child(body, cloned));
    assert_eq!(frames(&host, body), vec![imported_frame, clone_frame]);
    assert_eq!(
        host.owner_document_handle(clone_frame),
        Some(host.document_handle())
    );
}

#[test]
fn frame_candidates_exclude_inert_template_contents_and_foreign_elements() {
    let (mut host, body) = document();
    let template = host.create_element("template");
    let contents = host
        .node(template)
        .unwrap()
        .as_element()
        .unwrap()
        .template_contents()
        .unwrap();
    let inert_frame = host.create_element("iframe");
    assert!(host.append_child(contents, inert_frame));
    assert!(host.append_child(body, template));
    let outer = host.create_element("section");
    let shadow = host.attach_shadow_root(outer, "closed").unwrap();
    let foreign = host
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "iframe")
        .unwrap();
    let live_frame = host.create_element("iframe");
    assert!(host.append_child(shadow, foreign));
    assert!(host.append_child(shadow, live_frame));
    assert!(host.append_child(body, outer));
    host.compact_child_browsing_context_host_candidates();

    assert_eq!(frames(&host, body), vec![live_frame]);
    assert!(frames(&host, template).is_empty());
    assert_eq!(frames(&host, contents), vec![inert_frame]);
}
