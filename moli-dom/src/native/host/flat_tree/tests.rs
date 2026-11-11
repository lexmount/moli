use super::*;

fn test_host() -> DomHost {
    DomHost::from_dom(NativeDom::new_html(
        Url::parse("https://dialog.test/").unwrap(),
    ))
}

#[test]
fn removing_modal_subtrees_clears_registration_and_inertness() {
    for shadow in [false, true] {
        let mut host = test_host();
        let document = host.document_handle();
        let container = host.create_element("div");
        assert!(host.append_child(document, container));
        let parent = if shadow {
            host.attach_shadow_root(container, "open").unwrap()
        } else {
            container
        };
        let dialog = host.create_element("dialog");
        let background = host.create_element("button");
        assert!(host.append_child(parent, dialog));
        assert!(host.append_child(parent, background));
        host.set_attribute(dialog, "open", "");
        assert!(host.set_dialog_modal(dialog, true));
        assert_eq!(
            host.active_modal_dialog_for_document(document),
            Some(dialog)
        );
        assert!(host.is_inert_in_document(background));

        assert!(host.remove_child(document, container));
        assert!(
            !host
                .node(dialog)
                .unwrap()
                .as_element()
                .unwrap()
                .dialog_modal()
        );
        assert!(
            host.node(dialog)
                .unwrap()
                .as_element()
                .unwrap()
                .has_attribute("open")
        );
        assert!(
            host.node(document)
                .unwrap()
                .as_document()
                .unwrap()
                .modal_dialogs()
                .is_empty()
        );
        assert_eq!(host.active_modal_dialog_for_document(document), None);
        assert!(!host.is_inert_in_document(background));
    }
}

#[test]
fn modal_adoption_reopens_only_in_destination_document() {
    for explicit_removal in [false, true] {
        let mut host = test_host();
        let source = host.document_handle();
        let destination = host.create_detached_html_document();
        host.mark_subtree_connected_preserving_owner_document(destination);
        let dialog = host.create_element("dialog");
        let background = host.create_element("button");
        assert!(host.append_child(source, dialog));
        assert!(host.append_child(destination, background));
        host.set_attribute(dialog, "open", "");
        host.set_dialog_modal(dialog, true);
        if explicit_removal {
            assert!(host.remove_child(source, dialog));
        }
        assert!(host.append_child(destination, dialog));
        assert!(
            !host
                .node(dialog)
                .unwrap()
                .as_element()
                .unwrap()
                .dialog_modal()
        );
        assert!(
            host.node(source)
                .unwrap()
                .as_document()
                .unwrap()
                .modal_dialogs()
                .is_empty()
        );
        assert_eq!(host.active_modal_dialog_for_document(destination), None);

        host.remove_attribute(dialog, "open");
        host.set_attribute(dialog, "open", "");
        host.set_dialog_modal(dialog, true);
        assert_eq!(host.active_modal_dialog_for_document(source), None);
        assert_eq!(
            host.active_modal_dialog_for_document(destination),
            Some(dialog)
        );
        assert!(host.is_inert_in_document(background));
    }
}

#[test]
fn atomic_move_preserves_modal_opening_and_priority() {
    for shadow in [false, true] {
        let mut host = test_host();
        let document = host.document_handle();
        let container = host.create_element("div");
        let destination = host.create_element("section");
        assert!(host.append_child(document, container));
        assert!(host.append_child(document, destination));
        let parent = if shadow {
            host.attach_shadow_root(container, "open").unwrap()
        } else {
            container
        };
        let dialog = host.create_element("dialog");
        assert!(host.append_child(parent, dialog));
        host.set_attribute(dialog, "open", "");
        host.set_dialog_modal(dialog, true);
        let opening = host
            .node(dialog)
            .unwrap()
            .as_element()
            .unwrap()
            .dialog_modal_generation();
        let top = host.create_element("dialog");
        assert!(host.append_child(document, top));
        host.set_attribute(top, "open", "");
        host.set_dialog_modal(top, true);

        assert!(
            host.insert_before_preserving_state_effects(destination, container, None)
                .did_change()
        );
        assert_eq!(
            host.node(dialog)
                .unwrap()
                .as_element()
                .unwrap()
                .dialog_modal_generation(),
            opening
        );
        assert!(
            host.node(dialog)
                .unwrap()
                .as_element()
                .unwrap()
                .dialog_modal()
        );
        assert_eq!(host.active_modal_dialog_for_document(document), Some(top));
        host.set_dialog_modal(top, false);
        assert_eq!(
            host.active_modal_dialog_for_document(document),
            Some(dialog)
        );

        // appendChild is an ordinary remove/insert, even within one Document.
        assert!(host.append_child(document, container));
        assert!(
            !host
                .node(dialog)
                .unwrap()
                .as_element()
                .unwrap()
                .dialog_modal()
        );
        assert_eq!(host.active_modal_dialog_for_document(document), None);
    }
}

#[test]
fn removing_only_open_attribute_does_not_run_dialog_removing_steps() {
    let mut host = test_host();
    let document = host.document_handle();
    let dialog = host.create_element("dialog");
    assert!(host.append_child(document, dialog));
    host.set_attribute(dialog, "open", "");
    host.set_dialog_modal(dialog, true);
    host.remove_attribute(dialog, "open");
    assert!(
        host.node(dialog)
            .unwrap()
            .as_element()
            .unwrap()
            .dialog_modal()
    );
    assert_eq!(
        host.node(document)
            .unwrap()
            .as_document()
            .unwrap()
            .modal_dialogs(),
        &[dialog]
    );
}
