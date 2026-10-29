use super::*;
use crate::native::{DomHost, Node};

fn test_url() -> Url {
    Url::parse("https://metadata.test/source").unwrap()
}

#[test]
fn replacing_document_discards_foreign_template_owner_handles() {
    for source_padding in [0, 6] {
        let mut source = NativeDom::new_html(test_url());
        for _ in 0..source_padding {
            source.create_element("div");
        }
        source.create_element("template");

        // The foreign owner ID can alias the new Document, a fragment, an
        // unrelated element, or an unallocated slot in the receiving arena.
        for target_padding in [0, 1, 3] {
            let mut target = NativeDom::new_html(test_url());
            for _ in 0..target_padding {
                target.create_element("div");
            }
            let document = target.replace_main_document(source.document().unwrap());
            let fragment = target.create_template_contents_fragment_for_document(document, None);
            let owner = target.node(fragment).unwrap().owner_document().unwrap();
            let owner_document = target
                .node(owner)
                .and_then(Node::as_document)
                .expect("template contents must have a valid Document in the receiving arena");
            assert_ne!(owner, fragment);
            assert_ne!(owner, document);
            assert!(owner_document.is_inert_template_document());
            assert_eq!(
                target.appropriate_template_contents_owner_document(document),
                owner
            );
        }
    }
}

#[test]
fn replacing_document_preserves_metadata_without_arena_state() {
    for mut source in [
        Document::new_html(test_url()),
        Document::new_xml(test_url()),
        Document::new_inert_template(DocumentKind::Html),
        Document::new_inert_template(DocumentKind::Xml),
    ] {
        source.set_url(test_url());
        source.set_content_type("application/example+xml");
        source.set_character_set("windows-1252");
        source.set_ready_state(DocumentReadyState::Loading);
        source.set_quirks_mode(QuirksMode::LimitedQuirks);
        source.set_design_mode_enabled(true);
        source.set_default_language(Some("zh-CN".into()));
        source.set_source_last_modified_ms(Some(1234.0));
        source.set_base_url_override(Some(Url::parse("https://base.test/").unwrap()));
        source.set_css_target(Some(NativeNodeId::new(42)));
        source.set_associated_inert_template_document(NativeNodeId::new(43));

        let mut target = NativeDom::new_html(test_url());
        let document_handle = target.replace_main_document(&source);
        let document = target.document().unwrap();
        assert_eq!(document.url(), source.url());
        assert_eq!(document.kind(), source.kind());
        assert_eq!(document.interface_name(), source.interface_name());
        assert_eq!(document.content_type(), "application/example+xml");
        assert_eq!(document.character_set(), "windows-1252");
        assert_eq!(document.ready_state(), DocumentReadyState::Loading);
        assert_eq!(document.quirks_mode(), QuirksMode::LimitedQuirks);
        assert!(document.design_mode_enabled());
        assert_eq!(document.default_language(), Some("zh-CN"));
        assert_eq!(document.source_last_modified_ms(), Some(1234.0));
        assert_eq!(document.base_url().as_str(), "https://base.test/");
        assert_eq!(document.fallback_base_url(), source.fallback_base_url());
        assert!(!document.is_inert_template_document());
        assert_eq!(document.associated_inert_template_document(), None);
        assert_eq!(document.css_target(), None);

        let fragment = target.create_template_contents_fragment_for_document(document_handle, None);
        let owner = target.node(fragment).unwrap().owner_document().unwrap();
        let owner_document = target.node(owner).and_then(Node::as_document).unwrap();
        assert_ne!(owner, document_handle);
        assert_eq!(owner_document.kind(), source.kind());
        assert_eq!(owner_document.interface_name(), "Document");
        assert!(owner_document.is_inert_template_document());
    }
}

#[test]
fn document_node_clones_do_not_keep_the_original_css_target() {
    let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.append_child(document, target));
    assert!(host.set_document_target_element(document, Some(target)));

    for deep in [false, true] {
        let clone = host.clone_node(document, deep).unwrap();
        assert_eq!(
            host.node(clone)
                .and_then(Node::as_document)
                .unwrap()
                .css_target(),
            None
        );
    }
    assert_eq!(host.document_target_element(document), Some(target));
}

#[test]
fn document_node_clones_freeze_computed_base_url_without_tree_caches() {
    for deep in [false, true] {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_handle();
        let base = host.create_element("base");
        assert!(host.set_attribute(base, "href", "https://cdn.example/assets/"));
        assert!(host.set_attribute(base, "target", "source-window"));
        assert!(host.append_child(document, base));

        let clone = host.clone_node(document, deep).unwrap();
        if !deep {
            let cloned_document = host.node(clone).and_then(Node::as_document).unwrap();
            assert_eq!(cloned_document.base_element_url(), None);
            assert_eq!(cloned_document.base_target(), None);
        }
        if deep {
            let copied_base = host.node(clone).unwrap().first_child().unwrap();
            assert!(host.remove_child(clone, copied_base));
        }
        assert_eq!(
            host.document_base_url_for_handle(clone).unwrap().as_str(),
            "https://cdn.example/assets/"
        );

        let temporary_base = host.create_element("base");
        assert!(host.set_attribute(temporary_base, "href", "https://other.example/"));
        assert!(host.append_child(clone, temporary_base));
        assert_eq!(
            host.document_base_url_for_handle(clone).unwrap().as_str(),
            "https://other.example/"
        );
        assert!(host.remove_child(clone, temporary_base));
        assert_eq!(
            host.document_base_url_for_handle(clone).unwrap().as_str(),
            "https://cdn.example/assets/"
        );
        let cloned_document = host.node(clone).and_then(Node::as_document).unwrap();
        assert_eq!(cloned_document.base_element_url(), None);
        assert_eq!(cloned_document.base_target(), None);

        assert!(host.set_attribute(base, "href", "https://changed-source.example/"));
        assert_eq!(
            host.document_base_url_for_handle(clone).unwrap().as_str(),
            "https://cdn.example/assets/"
        );
        assert!(host.set_attribute(temporary_base, "href", "relative/"));
        assert!(host.append_child(clone, temporary_base));
        assert_eq!(
            host.document_base_url_for_handle(clone).unwrap().as_str(),
            "https://metadata.test/relative/"
        );
        assert!(host.remove_child(clone, temporary_base));
        let second_clone = host.clone_node(clone, false).unwrap();
        assert_eq!(
            host.document_base_url_for_handle(second_clone)
                .unwrap()
                .as_str(),
            "https://cdn.example/assets/"
        );
    }
}

#[test]
fn document_node_clones_preserve_about_base_url_separately_from_the_override() {
    let inherited = "https://parent.test/dir/page.html";
    for (url, about_base, relative_directory) in [
        (
            "about:blank",
            Some(inherited),
            Some("https://parent.test/dir/"),
        ),
        (
            "about:blank?query#source",
            Some(inherited),
            Some("https://parent.test/dir/"),
        ),
        (
            "about:srcdoc",
            Some(inherited),
            Some("https://parent.test/dir/"),
        ),
        ("about:srcdoc?query", Some(inherited), None),
        (
            "https://metadata.test/source",
            Some(inherited),
            Some("https://metadata.test/"),
        ),
        ("about:blank", None, None),
    ] {
        for deep in [false, true] {
            let mut host = DomHost::from_dom(NativeDom::new_html(Url::parse(url).unwrap()));
            let document = host.document_handle();
            host.set_document_fallback_base_url_for_handle(
                document,
                about_base.map(|base| Url::parse(base).unwrap()),
            );
            let fallback = host
                .node(document)
                .and_then(Node::as_document)
                .unwrap()
                .fallback_base_url()
                .clone();
            let base = host.create_element("base");
            assert!(host.set_attribute(base, "href", "https://cdn.example/assets/"));
            assert!(host.append_child(document, base));

            let clone = host.clone_node(document, deep).unwrap();
            assert_eq!(
                host.node(clone)
                    .and_then(Node::as_document)
                    .unwrap()
                    .fallback_base_url(),
                &fallback,
                "url={url}, deep={deep}"
            );
            if deep {
                let copied_base = host.node(clone).unwrap().first_child().unwrap();
                assert!(host.remove_child(clone, copied_base));
            }
            let relative_base = host.create_element("base");
            assert!(host.set_attribute(relative_base, "href", "fonts/"));
            assert!(host.append_child(clone, relative_base));
            let fonts_url =
                relative_directory.map_or_else(|| url.to_owned(), |base| format!("{base}fonts/"));
            assert_eq!(
                host.document_base_url_for_handle(clone).unwrap().as_str(),
                fonts_url,
                "url={url}, deep={deep}"
            );

            let repeated = host.clone_node(clone, false).unwrap();
            let second_base = host.create_element("base");
            assert!(host.set_attribute(second_base, "href", "images/"));
            assert!(host.append_child(repeated, second_base));
            let images_url =
                relative_directory.map_or_else(|| url.to_owned(), |base| format!("{base}images/"));
            assert_eq!(
                host.document_base_url_for_handle(repeated)
                    .unwrap()
                    .as_str(),
                images_url,
                "url={url}, deep={deep}"
            );
            assert!(host.remove_child(repeated, second_base));
            assert_eq!(
                host.document_base_url_for_handle(repeated)
                    .unwrap()
                    .as_str(),
                fonts_url
            );
            assert!(host.remove_child(clone, relative_base));
            assert_eq!(
                host.document_base_url_for_handle(clone).unwrap().as_str(),
                "https://cdn.example/assets/"
            );
        }
    }
}

#[test]
fn main_document_metadata_copy_does_not_freeze_a_base_element_override() {
    for kind in [DocumentKind::Html, DocumentKind::Xml] {
        for url in [
            "https://metadata.test/source",
            "about:blank",
            "about:srcdoc",
        ] {
            for override_url in [None, Some("https://override.test/frozen/")] {
                let url = Url::parse(url).unwrap();
                let mut source = match kind {
                    DocumentKind::Html => NativeDom::new_html(url.clone()),
                    DocumentKind::Xml => NativeDom::new_xml(url.clone()),
                };
                let document = source.document_node_id();
                let metadata = source
                    .node_mut(document)
                    .unwrap()
                    .data_mut()
                    .as_document_mut()
                    .unwrap();
                metadata.set_fallback_base_url(Some(
                    Url::parse("https://inherited.test/dir/page.html").unwrap(),
                ));
                metadata.set_base_url_override(override_url.map(|url| Url::parse(url).unwrap()));
                let expected_base = metadata.base_url().clone();
                let expected_fallback = metadata.fallback_base_url().clone();

                let base = source.create_element("base");
                assert!(source.set_attribute(base, "href", "https://cdn.example/assets/"));
                assert!(source.set_attribute(base, "target", "source-window"));
                assert!(source.append_child(document, base));
                let snapshot = source.clone();
                assert_eq!(
                    snapshot.document().unwrap().base_target(),
                    Some("source-window")
                );
                assert_eq!(
                    snapshot.document().unwrap().base_url().as_str(),
                    "https://cdn.example/assets/"
                );

                let mut target = NativeDom::new_html(test_url());
                let replacement = target.replace_main_document(source.document().unwrap());
                let copied = target.document().unwrap();
                assert_eq!(copied.url(), &url);
                assert_eq!(copied.kind(), kind);
                assert_eq!(copied.base_url(), &expected_base);
                assert_eq!(copied.fallback_base_url(), &expected_fallback);
                assert_eq!(copied.base_element_url(), None);
                assert_eq!(copied.base_target(), None);
                assert!(target.node(replacement).unwrap().first_child().is_none());
                assert_eq!(
                    source.document().unwrap().base_target(),
                    Some("source-window")
                );
                assert_eq!(
                    source.document().unwrap().base_url().as_str(),
                    "https://cdn.example/assets/"
                );
            }
        }
    }
}

#[test]
fn whole_dom_clones_preserve_arena_relationships_and_copy_on_write() {
    let mut source = NativeDom::new_html(test_url());
    let template = source.create_element("template");
    let document = source.document_node_id();
    let owner = source.appropriate_template_contents_owner_document(document);
    source
        .node_mut(document)
        .unwrap()
        .data_mut()
        .as_document_mut()
        .unwrap()
        .set_css_target(Some(template));
    let source_len = source.len();

    let mut snapshot = source.clone();
    assert_eq!(snapshot.document().unwrap().css_target(), Some(template));
    assert_eq!(
        snapshot.appropriate_template_contents_owner_document(document),
        owner
    );
    let fragment = snapshot.create_template_contents_fragment_for_document(owner, None);
    assert_eq!(
        snapshot.node(fragment).unwrap().owner_document(),
        Some(owner)
    );
    snapshot
        .node_mut(document)
        .unwrap()
        .data_mut()
        .as_document_mut()
        .unwrap()
        .set_css_target(None);

    assert_eq!(snapshot.document().unwrap().css_target(), None);
    assert_eq!(
        snapshot.appropriate_template_contents_owner_document(document),
        owner
    );
    assert!(
        snapshot
            .node(owner)
            .and_then(Node::as_document)
            .unwrap()
            .is_inert_template_document()
    );
    assert_eq!(source.len(), source_len);
    assert_eq!(source.document().unwrap().css_target(), Some(template));
    assert_eq!(
        source.appropriate_template_contents_owner_document(document),
        owner
    );
    assert!(source.node(fragment).is_none());
}
