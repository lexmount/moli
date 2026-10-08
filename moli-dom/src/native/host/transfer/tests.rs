use super::*;
use crate::native::{CustomElementState, DomStringValue, SelectedFile};

fn host(name: &str) -> DomHost {
    DomHost::from_dom(NativeDom::new_html(
        Url::parse(&format!("https://{name}.test/")).unwrap(),
    ))
}

fn element(host: &mut DomHost, handle: DomHandle) -> &mut Element {
    host.node_mut(handle)
        .unwrap()
        .data_mut()
        .as_element_mut()
        .unwrap()
}

#[test]
fn transfer_moves_complete_native_state_and_retires_source_handles() {
    let mut source = host("source");
    let mut target = host("target");
    for _ in 0..31 {
        target.create_element("aside");
    }
    let document = target.document_handle();
    let root = source.create_element("section");
    let input = source.create_element("input");
    element(&mut source, input)
        .set_input_value_from_user_edit(DomStringValue::from_utf16(&[0xd800, 65, 0xdfff]));
    element(&mut source, input).set_selection_range_with_direction(1, 2, "backward");
    element(&mut source, input).set_checked_with_dirty(true, true);
    let files = source.create_element("input");
    source.set_attribute(files, "type", "file");
    element(&mut source, files).set_selected_files(vec![SelectedFile {
        bytes: vec![1, 3, 9],
        mime_type: "text/plain".into(),
        name: "native.txt".into(),
        last_modified: 7.0,
    }]);
    let script = source.create_element("script");
    element(&mut source, script).set_script_already_started(true);
    element(&mut source, script).set_script_force_async(false);
    element(&mut source, script).set_script_text_internal_slot("retained source");
    let media = source.create_element("video");
    element(&mut source, media).set_media_current_time(41.0);
    element(&mut source, media).set_media_volume(0.25);
    element(&mut source, media).set_media_paused(false);
    let svg = source
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "p:svg")
        .unwrap();
    source.set_svg_root_scale(svg, 8.0);
    source.set_svg_root_translation(svg, [31.0, 7.0, 0.0, 1.0]);
    let custom = source.create_element("x-moved");
    element(&mut source, custom).set_custom_element_state(CustomElementState::Custom);
    element(&mut source, custom).set_custom_element_is_name(Some("x-moved".into()));
    element(&mut source, custom).set_cryptographic_nonce(Some("native nonce".into()));
    element(&mut source, custom).set_scroll_top(17.0);
    element(&mut source, custom).set_custom_validation_message("retained validation");
    element(&mut source, custom).set_attribute_ns_utf16_units(
        "value".into(),
        "urn:one".into(),
        Some("p".into()),
        "\u{fffd}".into(),
        vec![0xd800],
    );
    element(&mut source, custom).set_attribute_ns_utf16_units(
        "value".into(),
        "urn:two".into(),
        Some("p".into()),
        "\u{fffd}".into(),
        vec![0xdfff],
    );
    let text = source.create_text_node(DomStringValue::from_utf16(&[0xdfff, 13, 0xd800]));
    let children = [input, files, script, media, svg, custom, text];
    for child in children {
        assert!(source.append_child(root, child));
    }
    source
        .node_mut(script)
        .unwrap()
        .flags_mut()
        .set_parser_created(true);
    let payloads: HashMap<_, _> = [root]
        .into_iter()
        .chain(children)
        .map(|id| (id, format!("{:?}", source.node(id).unwrap().data())))
        .collect();
    let source_snapshot = source.clone();
    let allocated = source.dom.len();
    let source_version = source.query_version();
    let target_version = target.query_version();
    let moved = target
        .transfer_detached_subtree_from(document, &mut source, root)
        .unwrap();
    assert_eq!(moved.handles().len(), children.len() + 1);
    assert_eq!(source.dom.len(), allocated);
    assert_eq!(source.dom.nodes().len(), 1);
    assert!(source.query_version() > source_version);
    assert!(target.query_version() > target_version);
    for (&old, &new) in moved.handles() {
        assert!(source.node(old).is_none());
        let node = target.node(new).unwrap();
        assert_eq!(node.id(), new);
        assert_eq!(node.owner_document(), Some(document));
        assert!(!node.is_connected());
        assert_eq!(format!("{:?}", node.data()), payloads[&old]);
        assert!(source_snapshot.node(old).is_some());
    }
    assert_eq!(
        target.child_handles(moved.root()).collect::<Vec<_>>(),
        children.map(|id| moved.handles()[&id])
    );
    assert!(
        target
            .node(moved.handles()[&script])
            .unwrap()
            .flags()
            .parser_created()
    );
    let fresh = source.create_element("p");
    assert_eq!(fresh.index(), allocated);
    assert!(moved.handles().keys().all(|old| *old != fresh));
}

#[test]
fn transfer_preserves_all_shadow_roots_manual_slots_and_stylesheet_membership() {
    let mut source = host("source");
    let mut target = host("target");
    let root = source.create_element("div");
    let light = source.create_element("span");
    source.append_child(root, light);
    let mut init = ShadowRootInit::new("closed");
    init.set_slot_assignment("manual");
    init.set_serializable(true);
    init.set_null_custom_element_registry(true);
    let shadow = source
        .attach_declarative_shadow_root_with_init(root, init.clone())
        .unwrap();
    source.set_shadow_root_available_to_element_internals(shadow, true);
    let slot = source.create_element("slot");
    let style = source.create_element("style");
    source.append_child(shadow, slot);
    source.append_child(shadow, style);
    source.assign_nodes_to_slot(slot, vec![light]);
    let nested_host = source.create_element("div");
    source.append_child(shadow, nested_host);
    let nested = source.attach_shadow_root(nested_host, "open").unwrap();
    let content = source.create_text_node("nested");
    source.append_child(nested, content);
    assert_eq!(
        source
            .stylesheet_candidate_handles_for_tree_scope(shadow)
            .as_ref(),
        &[style]
    );
    let snapshot = source.clone();
    let moved = target
        .transfer_detached_subtree_from(target.document_handle(), &mut source, root)
        .unwrap();
    let new_root = moved.root();
    let new_shadow = moved.handles()[&shadow];
    assert_eq!(source.shadow_root_handle(root), None);
    assert_eq!(source.shadow_root_host(shadow), None);
    assert_eq!(
        source
            .stylesheet_candidate_handles_for_tree_scope(shadow)
            .len(),
        0
    );
    assert_eq!(target.shadow_root_handle(new_root), Some(new_shadow));
    assert_eq!(target.shadow_root_host(new_shadow), Some(new_root));
    assert_eq!(
        target.shadow_root_mode(new_shadow).as_deref(),
        Some("closed")
    );
    assert_eq!(target.shadow_root_clonable(new_shadow), Some(false));
    assert_eq!(target.shadow_root_serializable(new_shadow), Some(true));
    assert_eq!(target.shadow_root_is_declarative(new_shadow), Some(true));
    assert_eq!(
        target.shadow_root_available_to_element_internals(new_shadow),
        Some(true)
    );
    assert_eq!(
        target.shadow_root_uses_null_custom_element_registry(new_shadow),
        Some(true)
    );
    assert_eq!(
        target.assigned_nodes_for_slot_with_options(moved.handles()[&slot], false),
        vec![moved.handles()[&light]]
    );
    assert_eq!(
        target
            .stylesheet_candidate_handles_for_tree_scope(new_shadow)
            .as_ref(),
        &[moved.handles()[&style]]
    );
    assert_eq!(
        target.shadow_root_host(moved.handles()[&nested]),
        Some(moved.handles()[&nested_host])
    );
    assert_eq!(snapshot.shadow_root_handle(root), Some(shadow));
    assert_eq!(
        snapshot.assigned_nodes_for_slot_with_options(slot, false),
        vec![light]
    );
}

#[test]
fn transfer_retargets_nested_template_contents_to_target_inert_document() {
    for xml in [false, true] {
        let mut source = host("source");
        let mut target = if xml {
            DomHost::from_dom(NativeDom::new_xml(
                Url::parse("https://target.test/").unwrap(),
            ))
        } else {
            host("target")
        };
        let root = source.create_element("template");
        let contents = source
            .node(root)
            .unwrap()
            .as_element()
            .unwrap()
            .template_contents()
            .unwrap();
        let nested = source.create_element("template");
        let nested_contents = source
            .node(nested)
            .unwrap()
            .as_element()
            .unwrap()
            .template_contents()
            .unwrap();
        source.append_child(contents, nested);
        let text = source.create_text_node(DomStringValue::from_utf16(&[0xd800, 0xdfff]));
        source.append_child(nested_contents, text);
        let moved = target
            .transfer_detached_subtree_from(target.document_handle(), &mut source, root)
            .unwrap();
        let target_document = target.document_handle();
        let target_inert = target
            .dom
            .appropriate_template_contents_owner_document(target_document);
        assert_ne!(target_document, target_inert);
        assert_eq!(
            target.node(moved.root()).unwrap().owner_document(),
            Some(target_document)
        );
        for old in [contents, nested, nested_contents, text] {
            assert_eq!(
                target.node(moved.handles()[&old]).unwrap().owner_document(),
                Some(target_inert)
            );
        }
        assert_eq!(
            target
                .node(moved.handles()[&contents])
                .unwrap()
                .data()
                .as_document_fragment()
                .unwrap()
                .host(),
            Some(moved.root())
        );
        assert_eq!(
            target
                .node(moved.handles()[&nested])
                .unwrap()
                .as_element()
                .unwrap()
                .template_contents(),
            Some(moved.handles()[&nested_contents])
        );
        assert_eq!(
            target
                .node(moved.handles()[&text])
                .unwrap()
                .character_data_value()
                .unwrap()
                .utf16_units()
                .as_ref(),
            &[0xd800, 0xdfff]
        );
    }
}

#[test]
fn transfer_remaps_element_references_without_confusing_host_local_ids() {
    let mut source = host("source");
    let mut target = host("target");
    for _ in 0..40 {
        target.create_element("p");
    }
    let root = source.create_element("form");
    let input = source.create_element("input");
    let dialog = source.create_element("dialog");
    source.append_child(root, input);
    source.append_child(root, dialog);
    element(&mut source, input).set_parser_associated_form_owner(Some(root));
    element(&mut source, dialog).set_dialog_previously_focused_element(Some(input));
    element(&mut source, root)
        .set_explicit_element_references("aria-controls", vec![input, dialog, input]);
    let moved = target
        .transfer_detached_subtree_from(target.document_handle(), &mut source, root)
        .unwrap();
    let new_input = moved.handles()[&input];
    let new_dialog = moved.handles()[&dialog];
    assert_eq!(
        target
            .node(new_input)
            .unwrap()
            .as_element()
            .unwrap()
            .parser_associated_form_owner(),
        Some(moved.root())
    );
    assert_eq!(
        target
            .node(new_dialog)
            .unwrap()
            .as_element()
            .unwrap()
            .dialog_previously_focused_element(),
        Some(new_input)
    );
    assert_eq!(
        target
            .node(moved.root())
            .unwrap()
            .as_element()
            .unwrap()
            .explicit_element_references("aria-controls"),
        Some([new_input, new_dialog, new_input].as_slice())
    );
}

#[test]
fn transfer_rejects_open_references_before_mutating_either_host() {
    for incoming in [false, true] {
        let mut source = host("source");
        let mut target = host("target");
        let root = source.create_element("template");
        let other = source.create_element("p");
        let (node, referenced) = if incoming {
            (other, root)
        } else {
            (root, other)
        };
        element(&mut source, node)
            .set_explicit_element_references("aria-controls", vec![referenced]);
        let before_source = format!("{source:?}");
        let before_target = format!("{target:?}");
        assert_eq!(
            target.transfer_detached_subtree_from(target.document_handle(), &mut source, root),
            Err(DomSubtreeTransferError::OpenReference { node, referenced })
        );
        assert_eq!(format!("{source:?}"), before_source);
        assert_eq!(format!("{target:?}"), before_target);
    }
}

#[test]
fn transfer_rejects_connected_or_unsupported_sources_atomically() {
    let mut source = host("source");
    let mut target = host("target");
    let root = source.create_element("div");
    source.append_child(source.document_handle(), root);
    let before_source = format!("{source:?}");
    let before_target = format!("{target:?}");
    assert_eq!(
        target.transfer_detached_subtree_from(target.document_handle(), &mut source, root),
        Err(DomSubtreeTransferError::AttachedSource(root))
    );
    assert_eq!(format!("{source:?}"), before_source);
    assert_eq!(format!("{target:?}"), before_target);
    source.remove_child(source.document_handle(), root);
    let source_document = source.document_handle();
    assert_eq!(
        target.transfer_detached_subtree_from(
            target.document_handle(),
            &mut source,
            source_document
        ),
        Err(DomSubtreeTransferError::InvalidSource(source_document))
    );
    let shadow = source.attach_shadow_root(root, "closed").unwrap();
    assert_eq!(
        target.transfer_detached_subtree_from(target.document_handle(), &mut source, shadow),
        Err(DomSubtreeTransferError::InvalidSource(shadow))
    );
    let bad_document = target.create_element("p");
    assert_eq!(
        target.transfer_detached_subtree_from(bad_document, &mut source, root),
        Err(DomSubtreeTransferError::InvalidTargetDocument(bad_document))
    );
}

#[test]
fn transfer_keeps_retained_indexes_complete_across_repeated_moves() {
    let mut first = host("first");
    let mut second = host("second");
    let root = first.create_element("div");
    let child = first.create_element("p");
    first.set_attribute(child, "id", "moved");
    first.append_child(root, child);
    first.append_child(first.document_handle(), root);
    assert_eq!(first.element_handle_by_id("moved"), Some(child));
    first.remove_child(first.document_handle(), root);
    let frozen = first.clone();
    let mut current_root = root;
    let mut current_child = child;
    for _ in 0..3 {
        let moved = second
            .transfer_detached_subtree_from(second.document_handle(), &mut first, current_root)
            .unwrap();
        second.append_child(second.document_handle(), moved.root());
        assert_eq!(
            second.element_handle_by_id("moved"),
            Some(moved.handles()[&current_child])
        );
        assert_eq!(first.element_handle_by_id("moved"), None);
        second.remove_child(second.document_handle(), moved.root());
        let returned = first
            .transfer_detached_subtree_from(first.document_handle(), &mut second, moved.root())
            .unwrap();
        first.append_child(first.document_handle(), returned.root());
        let returned_child = returned.handles()[&moved.handles()[&current_child]];
        assert_eq!(first.element_handle_by_id("moved"), Some(returned_child));
        first.remove_child(first.document_handle(), returned.root());
        // Next iteration deliberately uses new identities, never vacant slots.
        assert!(first.node(current_root).is_none());
        assert!(second.node(moved.root()).is_none());
        current_root = returned.root();
        current_child = returned_child;
    }
    assert!(frozen.node(root).is_some());
    assert!(frozen.node(child).is_some());
}
