use moli_dom::native::{DomHost, DomStringValue, NativeDom};

fn host() -> DomHost {
    DomHost::from_dom(NativeDom::new_html(
        url::Url::parse("https://equality.test/").unwrap(),
    ))
}

#[test]
fn foreign_tree_equality_uses_each_nodes_own_arena_and_lossless_data() {
    let mut source = host();
    let root = source.create_element("div");
    let text = source.create_text_node("arena text");
    assert!(source.append_child(root, text));
    let comment = source.create_comment("comment");
    assert!(source.append_child(root, comment));
    let clone = source.clone_node(root, true).unwrap();
    assert!(
        source
            .node(root)
            .unwrap()
            .is_equal_node(source.dom(), source.node(clone).unwrap())
    );

    let mut target = host();
    for _ in 0..3 {
        target.create_comment("offset the target arena");
    }
    let imported = target
        .import_foreign_node(target.document_handle(), source.dom(), root, true)
        .unwrap();
    assert_ne!(
        root, imported,
        "the fixture must use different arena indexes"
    );
    assert!(source.node(root).unwrap().is_equal_node_in(
        source.dom(),
        target.node(imported).unwrap(),
        target.dom()
    ));
    assert!(target.node(imported).unwrap().is_equal_node_in(
        target.dom(),
        source.node(root).unwrap(),
        source.dom()
    ));

    let copied_text = target.child_handles(imported).next().unwrap();
    assert!(target.set_text_content(copied_text, "changed text"));
    assert!(!source.node(root).unwrap().is_equal_node_in(
        source.dom(),
        target.node(imported).unwrap(),
        target.dom()
    ));
    assert!(!target.node(imported).unwrap().is_equal_node_in(
        target.dom(),
        source.node(root).unwrap(),
        source.dom()
    ));
    let values = [
        DomStringValue::from_utf16(&[0xd800]),
        DomStringValue::from_utf16(&[0xd801]),
    ];
    assert_eq!(values[0].as_str_lossy(), values[1].as_str_lossy());
    let first = source.create_document_type(
        &values[0],
        DomStringValue::default(),
        DomStringValue::default(),
    );
    let copy = target
        .import_foreign_node(target.document_handle(), source.dom(), first, false)
        .unwrap();
    assert!(source.node(first).unwrap().is_equal_node_in(
        source.dom(),
        target.node(copy).unwrap(),
        target.dom()
    ));
    let second = target.create_document_type(
        &values[1],
        DomStringValue::default(),
        DomStringValue::default(),
    );
    assert!(!source.node(first).unwrap().is_equal_node_in(
        source.dom(),
        target.node(second).unwrap(),
        target.dom()
    ));
}
