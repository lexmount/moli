#[cfg(feature = "devtools")]
use crate::document_runtime::DomHandle;
#[cfg(feature = "devtools")]
use crate::dom::native::DomHost;

#[cfg(feature = "devtools")]
pub(super) fn child_frame_document_contains_live_handle(
    dom_host: &DomHost,
    document_handle: DomHandle,
    target: DomHandle,
) -> bool {
    collect_shadow_including_document_handles(dom_host, document_handle)
        .into_iter()
        .any(|handle| handle == target)
}

#[cfg(feature = "devtools")]
pub(super) fn collect_shadow_including_document_handles(
    dom_host: &DomHost,
    document_handle: DomHandle,
) -> Vec<DomHandle> {
    let mut handles = Vec::new();
    let mut stack = vec![document_handle];
    while let Some(handle) = stack.pop() {
        if dom_host.node(handle).is_none() {
            continue;
        }
        handles.push(handle);
        if let Some(shadow_root) = dom_host.shadow_root_handle(handle) {
            stack.push(shadow_root);
        }
        let children = dom_host.child_handles(handle).collect::<Vec<_>>();
        stack.extend(children.into_iter().rev());
    }
    handles
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "devtools")]
    use crate::dom::native::DomHost;
    use crate::dom::native::NativeDom;

    fn test_url() -> url::Url {
        url::Url::parse("https://example.test/").expect("test URL should parse")
    }

    #[test]
    fn shadow_including_handle_collection_walks_deep_tree_on_heap() {
        const DEEP_TREE_DEPTH: usize = 2048;

        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_handle();

        let mut parent = document;
        for _ in 0..DEEP_TREE_DEPTH {
            let child = host.create_element("div");
            assert!(host.append_child(parent, child));
            parent = child;
        }

        let handles = collect_shadow_including_document_handles(&host, document);

        assert_eq!(handles.len(), DEEP_TREE_DEPTH + 1);
        assert_eq!(handles.last().copied(), Some(parent));
    }

    #[test]
    fn shadow_including_handle_collection_observes_shadow_roots_within_limit() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_handle();
        let element = host.create_element("div");
        assert!(host.append_child(document, element));
        let shadow_root = host
            .attach_shadow_root(element, "open")
            .expect("shadow root should attach");

        let handles = collect_shadow_including_document_handles(&host, document);

        assert_eq!(handles, vec![document, element, shadow_root]);
    }
}
