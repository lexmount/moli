use super::ScriptVm;
use super::ScriptVmDefaultWorldBootstrap;
use super::StandaloneScriptVmHarness;
use crate::document_runtime::DomHandle;
use crate::dom::native::{DomHost, NativeDom};

fn new_vm() -> StandaloneScriptVmHarness {
    new_vm_with_url("https://detached-native.test/")
}

fn new_vm_with_url(url: &str) -> StandaloneScriptVmHarness {
    let _js_runtime = crate::JsRuntime::initialize();
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
    let page_runtime_task_source = page_task_queue.residence();
    ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(NativeDom::new(url::Url::parse(url).expect("test url"))),
        post_domcontentloaded_page_task_sender,
        page_task_front_injection_tx,
    )
    .expect("script vm bootstrap should succeed")
    .finish()
    .map(|mut vm| {
        vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
        vm
    })
    .expect("script vm finish should succeed")
}

fn eval(script: &str) -> String {
    let mut vm = new_vm();
    vm.eval(script).expect("script should evaluate")
}

fn eval_with_vm(script: &str) -> (String, StandaloneScriptVmHarness) {
    let mut vm = new_vm();
    let result = vm.eval(script).expect("script should evaluate");
    (result, vm)
}

fn element_handle_by_id(vm: &ScriptVm, id: &str) -> DomHandle {
    vm.document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            let element = node.as_element()?;
            (element.attribute("id") == Some(id)).then_some(DomHandle::new(index))
        })
        .unwrap_or_else(|| panic!("detached element #{id} should have a native handle"))
}

fn text_handle_by_value(vm: &ScriptVm, value: &str) -> DomHandle {
    vm.document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            (node.node_value() == Some(value)).then_some(DomHandle::new(index))
        })
        .unwrap_or_else(|| panic!("detached text node {value:?} should have a native handle"))
}

#[test]
fn create_html_document_shell_is_native_backed_for_traversal_identity() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const walker = document.createTreeWalker(d, NodeFilter.SHOW_ELEMENT);
            const html = walker.nextNode();
            const head = walker.nextNode();
            const title = walker.nextNode();
            const body = walker.nextNode();
            const ok = html === d.documentElement
                && head === d.head
                && title === d.head.firstChild
                && body === d.body
                && walker.nextNode() === null;
            return ok
                ? 'ok'
                : [
                    html && html.nodeName,
                    head && head.nodeName,
                    title && title.nodeName,
                    body && body.nodeName,
                    html === d.documentElement,
                    head === d.head,
                    title === d.head.firstChild,
                    body === d.body
                ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "ok");
}

#[test]
fn create_html_document_native_shell_preserves_optional_title_and_ownership() {
    let status = eval(
        r#"
        (() => {
            const untitled = document.implementation.createHTMLDocument();
            const titled = document.implementation.createHTMLDocument('');
            const nullTitle = document.implementation.createHTMLDocument(null);
            return [
                untitled.childNodes.length,
                untitled.doctype.name,
                untitled.head.childNodes.length,
                titled.head.childNodes.length,
                titled.head.firstChild.childNodes.length,
                titled.head.firstChild.firstChild.data,
                nullTitle.title,
                titled.documentElement instanceof HTMLHtmlElement,
                titled.head instanceof HTMLHeadElement,
                titled.head.firstChild instanceof HTMLTitleElement,
                titled.body instanceof HTMLBodyElement,
                titled.documentElement.ownerDocument === titled,
                titled.head.ownerDocument === titled,
                titled.body.ownerDocument === titled,
                titled.documentElement.parentNode === titled,
                titled.head.parentNode === titled.documentElement,
                titled.body.parentNode === titled.documentElement
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "2|html|0|1|1||null|true|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn create_html_document_reference_children_remain_attached_after_root_removal() {
    let status = eval(
        r#"
        (() => {
            const insertDoc = document.implementation.createHTMLDocument('title');
            insertDoc.removeChild(insertDoc.documentElement);
            const doctype = insertDoc.firstChild;
            const insertFragment = insertDoc.createDocumentFragment();
            insertFragment.append(insertDoc.createElement('a'), insertDoc.createElement('b'));
            let insertError = '';
            try {
                insertDoc.insertBefore(insertFragment, doctype);
            } catch (error) {
                insertError = error.name;
            }

            const replaceDoc = document.implementation.createHTMLDocument('title');
            const comment = replaceDoc.insertBefore(
                replaceDoc.createComment('before doctype'),
                replaceDoc.firstChild
            );
            replaceDoc.removeChild(replaceDoc.documentElement);
            const replacement = replaceDoc.createElement('replacement');
            let replaceError = '';
            try {
                replaceDoc.replaceChild(replacement, comment);
            } catch (error) {
                replaceError = error.name;
            }

            return [
                doctype === insertDoc.doctype,
                doctype.parentNode === insertDoc,
                comment.parentNode === replaceDoc,
                insertError,
                replaceError
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "true|true|true|HierarchyRequestError|HierarchyRequestError"
    );
}

#[test]
fn create_html_document_keeps_initial_child_wrappers_lazy() {
    let mut vm = new_vm();
    let retained_cache = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
            Ok(crate::native_bridge::identity::retain_context_wrapper_cache_for_test(scope))
        })
        .expect("wrapper cache should be retainable for regression testing");
    let baseline = retained_cache.wrapper_entry_count();

    let status = vm
        .eval(
            r#"
            globalThis.detachedDocumentForLazyShellTest =
                document.implementation.createHTMLDocument('');
            'ok';
            "#,
        )
        .expect("detached HTML document creation should evaluate");

    assert_eq!(status, "ok");
    assert_eq!(
        retained_cache.wrapper_entry_count(),
        baseline + 1,
        "only the returned Document wrapper should be materialized before its native shell is observed"
    );
}

#[test]
fn detached_document_created_nodes_keep_identity_through_native_traversal() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const div = d.createElement('div');
            const text = d.createTextNode('x');
            div.appendChild(text);
            d.body.appendChild(div);
            const iterator = document.createNodeIterator(d.body, NodeFilter.SHOW_ELEMENT);
            const body = iterator.nextNode();
            const found = iterator.nextNode();
            return body === d.body
                && found === div
                && found.firstChild === text
                && iterator.nextNode() === null
                    ? 'ok'
                    : 'bad';
        })()
        "#,
    );
    assert_eq!(status, "ok");
}

#[test]
fn adopted_detached_node_keeps_identity_through_live_child_nodes() {
    let status = eval(
        r#"
        (() => {
            const xmlDoc = document.implementation.createDocument(null, null);
            const xmlElement = xmlDoc.createElement('source');
            const xmlText = xmlDoc.createTextNode('payload');
            xmlElement.appendChild(xmlText);

            const host = document.createElement('p');
            host.textContent = 'prefix';
            host.appendChild(xmlText);

            return [
                host.childNodes[1] === xmlText,
                host.childNodes.item(1) === xmlText,
                Array.from(host.childNodes)[1] === xmlText,
                xmlText.parentNode === host
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true");
}

#[test]
fn adopted_detached_node_uses_one_identity_across_document_return_paths() {
    let status = eval(
        r#"
        (() => {
            const html = document.documentElement
                || document.appendChild(document.createElement('html'));
            const body = document.body
                || html.appendChild(document.createElement('body'));
            const detached = document.implementation.createHTMLDocument('');
            const node = detached.createElement('button');
            node.id = 'adopted-canonical-wrapper';
            body.appendChild(node);
            node.focus();

            return [
                document.getElementById(node.id) === node,
                document.querySelector('#adopted-canonical-wrapper') === node,
                document.activeElement === node,
                node.ownerDocument === document,
                body.lastChild === node
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true|true");
}

#[test]
fn detached_document_shell_builders_create_native_parent_links() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const htmlDoc = document.implementation.createHTMLDocument('');
            htmlDoc.documentElement.id = 'builder-native-html';
            htmlDoc.head.id = 'builder-native-head';
            htmlDoc.body.id = 'builder-native-body';
            const xmlDoc = document.implementation.createDocument('urn:test', 'root', null);
            xmlDoc.documentElement.setAttribute('id', 'builder-native-xml-root');
            return [
                htmlDoc.documentElement.parentNode === htmlDoc,
                htmlDoc.head.parentNode === htmlDoc.documentElement,
                htmlDoc.body.parentNode === htmlDoc.documentElement,
                xmlDoc.documentElement.parentNode === xmlDoc
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true");

    let html = element_handle_by_id(&vm, "builder-native-html");
    let head = element_handle_by_id(&vm, "builder-native-head");
    let body = element_handle_by_id(&vm, "builder-native-body");
    let xml_root = element_handle_by_id(&vm, "builder-native-xml-root");
    let dom = vm.document_runtime.dom_host().dom();
    assert_eq!(dom.parent_node(head), Some(html));
    assert_eq!(dom.parent_node(body), Some(html));
    assert!(
        dom.parent_node(html)
            .and_then(|handle| dom.node(handle))
            .is_some_and(|node| node.is_document())
    );
    assert!(
        dom.parent_node(xml_root)
            .and_then(|handle| dom.node(handle))
            .is_some_and(|node| node.is_document())
    );
}

#[test]
fn detached_document_shell_child_lists_materialize_from_native_tree() {
    let status = eval(
        r#"
        (() => {
            const htmlDoc = document.implementation.createHTMLDocument('');
            const xmlDoc = document.implementation.createDocument('urn:test', 'root', null);
            const htmlChildren = htmlDoc.childNodes;
            const rootChildren = htmlDoc.documentElement.childNodes;
            const xmlChildren = xmlDoc.childNodes;
            return [
                htmlDoc.doctype && htmlDoc.doctype.name,
                htmlChildren.length,
                htmlChildren[0] === htmlDoc.doctype,
                htmlChildren[1] === htmlDoc.documentElement,
                rootChildren.length,
                rootChildren[0] === htmlDoc.head,
                rootChildren[1] === htmlDoc.body,
                xmlDoc.doctype === null,
                xmlChildren.length,
                xmlChildren[0] === xmlDoc.documentElement
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "html|2|true|true|2|true|true|true|1|true");
}

#[test]
fn detached_xhtml_document_uses_native_root_namespace_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const xhtml = 'http://www.w3.org/1999/xhtml';
            const d = document.implementation.createDocument(xhtml, 'html', null);
            let tampered = false;
            Object.defineProperty(d.documentElement, 'namespaceURI', {
                configurable: true,
                get() {
                    tampered = true;
                    return 'urn:tampered';
                }
            });
            const child = d.createElement('span');
            return [
                d.contentType,
                child.namespaceURI,
                tampered
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "application/xhtml+xml|http://www.w3.org/1999/xhtml|false"
    );
}

#[test]
fn detached_document_url_query_encoding_does_not_inherit_main_document_legacy_charset() {
    let mut vm = new_vm();
    vm.document_runtime
        .set_document_character_set("windows-1252");

    let status = vm
        .eval(
            r#"
            (() => {
                const htmlDoc = document.implementation.createHTMLDocument('');
                const htmlAnchor = htmlDoc.createElement('a');
                htmlAnchor.href = 'http://example.org/?ä';

                const constructedDoc = new Document();
                const constructedAnchor = constructedDoc.createElementNS(
                    'http://www.w3.org/1999/xhtml',
                    'a'
                );
                constructedAnchor.href = 'http://example.org/?ä';

                const liveAnchor = document.createElement('a');
                liveAnchor.href = 'http://example.org/?ä';

                return [
                    htmlAnchor.href,
                    constructedAnchor.href,
                    liveAnchor.href
                ].join('|');
            })()
            "#,
        )
        .expect("detached document URL query encoding should evaluate");

    assert_eq!(
        status,
        "http://example.org/?%C3%A4|http://example.org/?%C3%A4|http://example.org/?%E4"
    );
}

#[test]
fn detached_fragment_insertion_updates_native_traversal_tree() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const fragment = d.createDocumentFragment();
            const section = d.createElement('section');
            const span = d.createElement('span');
            section.appendChild(span);
            fragment.appendChild(section);
            d.body.appendChild(fragment);
            const walker = document.createTreeWalker(d.body, NodeFilter.SHOW_ELEMENT);
            return walker.nextNode() === section
                && walker.nextNode() === span
                && walker.nextNode() === null
                && fragment.firstChild === null
                    ? 'ok'
                    : 'bad';
        })()
        "#,
    );
    assert_eq!(status, "ok");
}

#[test]
fn detached_shadow_root_inner_html_uses_native_child_tree() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const host = d.createElement('div');
            host.id = 'shadow-native-host';
            d.body.appendChild(host);
            const root = host.attachShadow({ mode: 'open' });
            root.innerHTML = '<span id="old-shadow-child"></span><b></b>';
            const projected = root.childNodes;
            root.innerHTML = '<em id="fresh-shadow-child"></em>';
            return [
                host.shadowRoot === root,
                projected.length,
                projected[0].id,
                projected[0].parentNode === root,
                root.firstChild.id,
                root.firstChild.parentNode === root
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "true|1|fresh-shadow-child|true|fresh-shadow-child|true"
    );

    let host = element_handle_by_id(&vm, "shadow-native-host");
    let fresh = element_handle_by_id(&vm, "fresh-shadow-child");
    let dom_host = vm.document_runtime.dom_host();
    let shadow_root = dom_host
        .shadow_root_handle(host)
        .expect("detached shadow root should have a native handle");
    let dom = dom_host.dom();
    assert_eq!(dom.first_child(shadow_root), Some(fresh));
    assert_eq!(dom.parent_node(fresh), Some(shadow_root));
}

#[test]
fn detached_child_nodes_reads_ignore_stale_projection_entries() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('section');
            parent.setAttribute('id', 'detached-childnodes-native');
            const real = d.createElement('span');
            real.setAttribute('id', 'native-child');
            parent.appendChild(real);
            d.body.appendChild(parent);
            const projected = parent.childNodes;
            projected[1] = { nodeType: 1, id: 'stale-child' };
            projected.length = 2;
            const refreshed = parent.childNodes;
            return [
                refreshed.length,
                refreshed[0] === real,
                refreshed[1],
                parent.firstChild === real,
                parent.lastChild === real
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "1|true||true|true");

    let parent = element_handle_by_id(&vm, "detached-childnodes-native");
    let real = element_handle_by_id(&vm, "native-child");
    let dom = vm.document_runtime.dom_host().dom();
    assert_eq!(dom.first_child(parent), Some(real));
    assert_eq!(dom.next_sibling(real), None);
}

#[test]
fn detached_native_child_nodes_are_live_native_node_lists() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('section');
            const first = d.createElement('span');
            const second = d.createElement('em');
            const fragment = d.createDocumentFragment();
            const fragmentChild = d.createElement('strong');
            fragmentChild.id = 'frag';
            second.setAttribute('name', 'secondName');
            d.body.appendChild(parent);
            const parentNodes = parent.childNodes;
            const parentChildren = parent.children;
            const fragmentNodes = fragment.childNodes;
            const fragmentChildren = fragment.children;
            fragment.appendChild(fragmentChild);
            parent.appendChild(first);
            parent.insertBefore(fragment, first);
            parent.appendChild(second);
            parent.removeChild(first);
            return [
                parentNodes.length,
                parentNodes[0] === fragmentChild,
                parentNodes[1] === second,
                parent.firstChild === fragmentChild,
                parent.lastChild === second,
                fragmentNodes.length,
                fragmentChild.parentNode === parent,
                first.parentNode === null,
                parent.childNodes === parentNodes,
                fragment.childNodes === fragmentNodes,
                parentChildren.length,
                parentChildren[0] === fragmentChild,
                parentChildren[1] === second,
                parentChildren.item(1) === second,
                parentChildren.namedItem('frag') === fragmentChild,
                parentChildren.secondName === second,
                parent.children === parentChildren,
                fragment.children === fragmentChildren,
                fragmentChildren.length
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "2|true|true|true|true|0|true|true|true|true|2|true|true|true|true|true|true|true|0"
    );
}

#[test]
fn detached_attribute_mutations_sync_to_native_handle() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            const observer = new MutationObserver(() => {});
            observer.observe(node, { attributes: true, attributeOldValue: true });
            node.id = 'detached-attr-sync';
            node.setAttribute('data-temp', 'remove-me');
            node.setAttributeNS('urn:attr', 'a:flag', 'one');
            node.setAttributeNS('urn:attr', 'b:flag', 'two');
            node.removeAttribute('data-temp');
            d.body.appendChild(node);
            const records = observer.takeRecords()
                .map((record) => [
                    record.attributeName,
                    record.attributeNamespace || '',
                    record.oldValue,
                    record.target === node
                ].join(':'))
                .join(',');
            return [
                node.getAttribute('id'),
                node.getAttribute('a:flag'),
                node.getAttribute('b:flag'),
                node.getAttributeNS('urn:attr', 'flag'),
                node.hasAttribute('data-temp'),
                records
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "detached-attr-sync|two||two|false|id:::true,data-temp:::true,flag:urn:attr::true,flag:urn:attr:one:true,data-temp::remove-me:true"
    );

    let dom_host = vm.document_runtime.dom_host();
    let handle = dom_host
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            let element = node.as_element()?;
            (element.local_name() == "p" && element.attribute("id") == Some("detached-attr-sync"))
                .then_some(DomHandle::new(index))
        })
        .expect("detached element should have a native handle");
    assert_eq!(
        dom_host.get_attribute_ns(handle, Some("urn:attr"), "flag"),
        Some("two".to_owned())
    );
    assert_eq!(
        dom_host.get_attribute(handle, "a:flag"),
        Some("two".to_owned())
    );
    assert_eq!(dom_host.get_attribute(handle, "b:flag"), None);
    assert_eq!(dom_host.get_attribute(handle, "data-temp"), None);
}

#[test]
fn detached_html_attribute_names_use_native_namespace_after_property_tamper() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.id = 'detached-attr-name-native-namespace';
            d.body.appendChild(node);
            Object.defineProperty(node, 'namespaceURI', {
                get() { return 'urn:spoofed'; },
                configurable: true
            });
            node.setAttribute('DATA-CASE', 'value');
            const attr = node.getAttributeNode('DATA-CASE');
            const beforeRemove = [
                node.getAttribute('data-case'),
                node.getAttribute('DATA-CASE'),
                node.hasAttribute('data-case'),
                attr && attr.name,
                attr && attr.value,
                node.getAttributeNames().includes('data-case'),
                node.getAttributeNames().includes('DATA-CASE')
            ].join('|');
            node.removeAttribute('DATA-CASE');
            return [
                beforeRemove,
                node.hasAttribute('data-case'),
                node.getAttribute('DATA-CASE'),
                node.getAttributeNames().includes('data-case')
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "value|value|true|data-case|value|true|false|false||false"
    );

    let handle = element_handle_by_id(&vm, "detached-attr-name-native-namespace");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(dom_host.get_attribute(handle, "data-case"), None);
    assert_eq!(dom_host.get_attribute(handle, "DATA-CASE"), None);
}

#[test]
fn detached_form_control_paths_use_native_local_name_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = new DOMParser().parseFromString(
                '<html><body>' +
                    '<select id="select">' +
                        '<option value="first-native" selected>First</option>' +
                        '<option value="second-native">Second</option>' +
                    '</select>' +
                    '<textarea id="textarea">initial-native</textarea>' +
                '</body></html>',
                'text/html'
            );
            const select = d.getElementById('select');
            const textarea = d.getElementById('textarea');
            for (const node of [select, textarea]) {
                Object.defineProperty(node, 'localName', {
                    get() { return 'div'; },
                    configurable: true
                });
            }

            const selectBefore = select.value;
            select.value = 'second-native';
            delete select.localName;
            const selectAfter = select.value;
            const textareaBefore = textarea.value;
            textarea.defaultValue = 'changed-native';
            const textareaText = textarea.textContent;
            return [
                selectBefore,
                selectAfter,
                textareaBefore,
                textareaText
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "first-native|second-native|initial-native|changed-native"
    );
}

#[test]
fn detached_attribute_removals_read_from_native_after_state_drift() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.setAttribute('id', 'detached-attr-remove-native');
            node.setAttribute('data-remove', 'plain');
            node.setAttributeNS('urn:remove', 'r:flag', 'namespaced');
            d.body.appendChild(node);
            node.removeAttribute('data-remove');
            node.removeAttributeNS('urn:remove', 'flag');
            return [
                node.getAttribute('data-remove'),
                node.hasAttribute('data-remove'),
                node.getAttributeNS('urn:remove', 'flag'),
                node.hasAttributeNS('urn:remove', 'flag'),
                node.getAttributeNames().includes('data-remove'),
                node.getAttributeNames().includes('r:flag')
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "|false||false|false|false");

    let handle = element_handle_by_id(&vm, "detached-attr-remove-native");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(dom_host.get_attribute(handle, "data-remove"), None);
    assert_eq!(
        dom_host.get_attribute_ns(handle, Some("urn:remove"), "flag"),
        None
    );
}

#[test]
fn detached_attribute_maps_ignore_public_map_constructor() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const intrinsicMap = Map;
            globalThis.Map = function() {
                throw new Error('detached DOM must not construct the public Map');
            };
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.setAttribute('id', 'detached-map-primordial');
            node.setAttribute('data-value', 'plain');
            node.setAttributeNS('urn:map', 'm:flag', 'namespaced');
            d.body.appendChild(node);
            const result = [
                node.getAttribute('data-value'),
                node.getAttributeNS('urn:map', 'flag'),
                node.getAttributeNames().join(','),
                node.attributes.length
            ].join('|');
            globalThis.Map = intrinsicMap;
            return result;
        })()
        "#,
    );
    assert_eq!(status, "plain|namespaced|id,data-value,m:flag|3");

    let handle = element_handle_by_id(&vm, "detached-map-primordial");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(
        dom_host.get_attribute(handle, "data-value"),
        Some("plain".to_owned())
    );
    assert_eq!(
        dom_host.get_attribute_ns(handle, Some("urn:map"), "flag"),
        Some("namespaced".to_owned())
    );
}

#[test]
fn detached_attribute_writes_use_native_handle_after_state_drift() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.setAttribute('id', 'detached-attr-write-native');
            node.setAttributeNS('urn:set', 'a:flag', 'old');
            d.body.appendChild(node);
            Map.prototype.set = function() {
                throw new Error('legacy attribute maps should not be written');
            };
            node.setAttribute('data-written', 'plain');
            node.setAttributeNS('urn:set', 'b:flag', 'new');
            return [
                node.getAttribute('data-written'),
                node.getAttribute('a:flag'),
                node.getAttribute('b:flag'),
                node.getAttributeNS('urn:set', 'flag'),
                node.getAttributeNames().includes('a:flag'),
                node.getAttributeNames().includes('b:flag'),
                node.attributes.length,
                node.attributes[2].name,
                node.attributes[2].value
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "plain|new||new|true|false|3|data-written|plain");

    let handle = element_handle_by_id(&vm, "detached-attr-write-native");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(
        dom_host.get_attribute(handle, "data-written"),
        Some("plain".to_owned())
    );
    assert_eq!(
        dom_host.get_attribute(handle, "a:flag"),
        Some("new".to_owned())
    );
    assert_eq!(dom_host.get_attribute(handle, "b:flag"), None);
    assert_eq!(
        dom_host.get_attribute_ns(handle, Some("urn:set"), "flag"),
        Some("new".to_owned())
    );
}

#[test]
fn detached_attribute_node_methods_use_native_handle_after_state_drift() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.setAttribute('id', 'detached-attr-node-native');
            d.body.appendChild(node);
            const plain = d.createAttribute('data-node');
            plain.value = 'one';
            const replacement = d.createAttribute('data-node');
            replacement.value = 'two';
            const namespaced = d.createAttributeNS('urn:node', 'n:flag');
            namespaced.value = 'ns';
            const elementPrototype = Object.getPrototypeOf(node);
            elementPrototype.setAttribute = function() {
                throw new Error('setAttribute should not be used');
            };
            elementPrototype.removeAttribute = function() {
                throw new Error('removeAttribute should not be used');
            };
            Map.prototype.set = function() {
                throw new Error('legacy attribute maps should not be written');
            };
            Map.prototype.delete = function() {
                throw new Error('legacy attribute maps should not be deleted');
            };
            const firstOld = node.setAttributeNode(plain);
            const secondOld = node.setAttributeNode(replacement);
            node.setAttributeNode(namespaced);
            const removed = node.removeAttributeNode(replacement);
            return [
                firstOld === null,
                secondOld === plain,
                plain.ownerElement === null,
                replacement.ownerElement === null,
                removed === replacement,
                node.getAttribute('data-node'),
                node.getAttributeNS('urn:node', 'flag'),
                namespaced.ownerElement === node,
                node.attributes.length,
                node.attributes[1].name,
                node.attributes[1].value
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true|true||ns|true|2|n:flag|ns");

    let handle = element_handle_by_id(&vm, "detached-attr-node-native");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(dom_host.get_attribute(handle, "data-node"), None);
    assert_eq!(
        dom_host.get_attribute_ns(handle, Some("urn:node"), "flag"),
        Some("ns".to_owned())
    );
}

#[test]
fn detached_reflected_element_attributes_use_native_handle_after_method_tamper() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('a');
            d.body.appendChild(node);
            Object.defineProperty(node, 'getAttribute', {
                value() {
                    throw new Error('getAttribute should not be called');
                },
                configurable: true
            });
            Object.defineProperty(node, 'setAttribute', {
                value() {
                    throw new Error('setAttribute should not be called');
                },
                configurable: true
            });
            node.id = 'reflected-native';
            node.className = 'native-class';
            return [
                node.id,
                node.className
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "reflected-native|native-class");

    let handle = element_handle_by_id(&vm, "reflected-native");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(
        dom_host.get_attribute(handle, "class").as_deref(),
        Some("native-class")
    );
}

#[test]
fn detached_set_attribute_node_detaches_replaced_native_attr_object() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.setAttribute('data-node', 'one');
            const old = node.getAttributeNode('data-node');
            const replacement = d.createAttribute('data-node');
            replacement.value = 'two';
            const returned = node.setAttributeNode(replacement);
            return [
                returned === old,
                old.ownerElement === null,
                old.value,
                replacement.ownerElement === node,
                node.getAttribute('data-node')
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|one|true|two");
}

#[test]
fn detached_remove_attribute_ns_detaches_cached_native_attr_aliases() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.setAttributeNS('urn:remove-cache', 'r:flag', 'namespaced');
            const nsAttr = node.getAttributeNodeNS('urn:remove-cache', 'flag');
            const namedAttr = node.getAttributeNode('r:flag');
            node.removeAttributeNS('urn:remove-cache', 'flag');
            return [
                nsAttr === namedAttr,
                nsAttr.ownerElement === null,
                namedAttr.ownerElement === null,
                nsAttr.value,
                namedAttr.value,
                node.getAttributeNS('urn:remove-cache', 'flag'),
                node.getAttributeNodeNS('urn:remove-cache', 'flag'),
                node.getAttributeNode('r:flag')
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|namespaced|namespaced|||");
}

#[test]
fn detached_attr_value_setter_uses_native_handle_after_state_drift() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.setAttribute('id', 'detached-attr-value-native');
            const plain = d.createAttribute('data-value');
            plain.value = 'old';
            const namespaced = d.createAttributeNS('urn:value', 'v:flag');
            namespaced.value = 'old-ns';
            node.setAttributeNode(plain);
            node.setAttributeNode(namespaced);
            d.body.appendChild(node);
            const elementPrototype = Object.getPrototypeOf(node);
            elementPrototype.setAttribute = function() {
                throw new Error('setAttribute should not be used');
            };
            elementPrototype.setAttributeNS = function() {
                throw new Error('setAttributeNS should not be used');
            };
            Map.prototype.set = function() {
                throw new Error('legacy attribute maps should not be written');
            };
            plain.value = 'new';
            namespaced.value = 'new-ns';
            return [
                node.getAttribute('data-value'),
                node.getAttributeNS('urn:value', 'flag'),
                plain.value,
                namespaced.value,
                node.attributes.length
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "new|new-ns|new|new-ns|3");

    let handle = element_handle_by_id(&vm, "detached-attr-value-native");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(
        dom_host.get_attribute(handle, "data-value"),
        Some("new".to_owned())
    );
    assert_eq!(
        dom_host.get_attribute_ns(handle, Some("urn:value"), "flag"),
        Some("new-ns".to_owned())
    );
}

#[test]
fn detached_attribute_reads_prefer_native_handle() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const node = d.createElement('p');
                node.setAttribute('id', 'detached-native-read');
                node.setAttribute('data-origin', 'js-state');
                node.setAttributeNS('urn:read', 'r:flag', 'js-state');
                d.body.appendChild(node);
                globalThis.__detachedNativeReadNode = node;
                return node.getAttribute('data-origin') + '|' + node.getAttributeNS('urn:read', 'flag');
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "js-state|js-state");

    let handle = vm
        .document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            let element = node.as_element()?;
            (element.local_name() == "p" && element.attribute("id") == Some("detached-native-read"))
                .then_some(DomHandle::new(index))
        })
        .expect("detached element should have a native handle");
    let dom_host = vm.document_runtime.dom_host_mut();
    assert!(dom_host.set_attribute(handle, "data-origin", "native-state"));
    assert!(dom_host.set_attribute(handle, "data-added", "native-only"));
    assert!(dom_host.set_attribute_ns(
        handle,
        Some("urn:read"),
        Some("native"),
        "flag",
        "native-state",
    ));

    let status = vm
        .eval(
            r#"
            (() => {
                const node = globalThis.__detachedNativeReadNode;
                return [
                    node.getAttribute('data-origin'),
                    node.getAttribute('data-added'),
                    node.getAttributeNS('urn:read', 'flag'),
                    node.hasAttribute('data-added'),
                    node.getAttributeNames().includes('data-added')
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed reads should evaluate");
    assert_eq!(status, "native-state|native-only|native-state|true|true");
}

#[test]
fn detached_named_node_map_uses_native_attributes_after_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const empty = d.createElement('p');
            const populated = d.createElement('section');
            populated.setAttribute('data-real', 'native');
            for (const node of [empty, populated]) {
                node.getAttributeNames = () => ['data-fake'];
                node.getAttributeNode = name => ({ name, value: 'fake', ownerElement: node });
            }
            const emptyAttrs = empty.attributes;
            const populatedAttrs = populated.attributes;
            return [
                emptyAttrs.length,
                emptyAttrs.item(0) === null,
                emptyAttrs[0] === undefined,
                emptyAttrs.getNamedItem('data-fake') === null,
                emptyAttrs['data-fake'] === undefined,
                Object.keys(emptyAttrs).join(','),
                populatedAttrs.length,
                populatedAttrs.item(0).name,
                populatedAttrs.item(0).value,
                populatedAttrs.getNamedItem('data-real').value,
                populatedAttrs.getNamedItem('data-fake') === null,
                populatedAttrs['data-real'].value,
                populatedAttrs['data-fake'] === undefined
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "0|true|true|true|true||1|data-real|native|native|true|native|true"
    );
}

#[test]
fn detached_named_node_map_named_properties_use_native_namespace_after_property_tamper() {
    let (setup, mut vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.id = 'detached-named-node-map-native-namespace';
            d.body.appendChild(node);
            globalThis.__detachedNamedNodeMapNamespaceNode = node;
            return node.namespaceURI;
        })()
        "#,
    );
    assert_eq!(setup, "http://www.w3.org/1999/xhtml");

    let handle = element_handle_by_id(&vm, "detached-named-node-map-native-namespace");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .set_attribute(handle, "DATA-UPPER", "native")
    );

    let status = vm
        .eval(
            r#"
            (() => {
                const node = globalThis.__detachedNamedNodeMapNamespaceNode;
                let tampered = false;
                Object.defineProperty(node, 'namespaceURI', {
                    configurable: true,
                    get() {
                        tampered = true;
                        return 'urn:spoofed';
                    }
                });
                return [
                    node.attributes.length,
                    node.attributes.item(0).name,
                    node.attributes.item(0).value,
                    Object.keys(node.attributes).includes('DATA-UPPER'),
                    tampered
                ].join('|');
            })()
            "#,
        )
        .expect("named node map namespace tamper test should evaluate");
    assert_eq!(
        status,
        "2|id|detached-named-node-map-native-namespace|false|false"
    );
}

#[test]
fn detached_named_node_map_mutations_use_native_attributes_after_method_tamper() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const node = d.createElement('p');
            node.setAttribute('id', 'detached-named-node-map-native-mutation');
            d.body.appendChild(node);

            const plain = d.createAttribute('data-map');
            plain.value = 'one';
            const replacement = d.createAttribute('data-map');
            replacement.value = 'two';
            const namespaced = d.createAttributeNS('urn:map', 'm:flag');
            namespaced.value = 'ns';

            const elementPrototype = Object.getPrototypeOf(node);
            elementPrototype.setAttributeNode = function() {
                throw new Error('setAttributeNode should not be used');
            };
            elementPrototype.removeAttributeNode = function() {
                throw new Error('removeAttributeNode should not be used');
            };
            elementPrototype.removeAttributeNS = function() {
                throw new Error('removeAttributeNS should not be used');
            };
            node.getAttributeNode = function() {
                throw new Error('getAttributeNode should not be used');
            };
            node.getAttributeNames = () => ['data-fake'];
            Map.prototype.set = function() {
                throw new Error('legacy attribute maps should not be written');
            };
            Map.prototype.delete = function() {
                throw new Error('legacy attribute maps should not be deleted');
            };

            const attrs = node.attributes;
            const firstOld = attrs.setNamedItem(plain);
            const secondOld = attrs.setNamedItem(replacement);
            const nsOld = attrs.setNamedItemNS(namespaced);
            const removedPlain = attrs.removeNamedItem('data-map');
            const removedNs = attrs.removeNamedItemNS('urn:map', 'flag');
            return [
                firstOld === null,
                secondOld === plain,
                nsOld === null,
                removedPlain === replacement,
                removedNs === namespaced,
                plain.ownerElement === null,
                replacement.ownerElement === null,
                namespaced.ownerElement === null,
                node.getAttribute('data-map'),
                node.getAttributeNS('urn:map', 'flag'),
                attrs.length,
                attrs.getNamedItem('data-fake') === null
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true|true|true|true|true|||1|true");

    let handle = element_handle_by_id(&vm, "detached-named-node-map-native-mutation");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(dom_host.get_attribute(handle, "data-map"), None);
    assert_eq!(
        dom_host.get_attribute_ns(handle, Some("urn:map"), "flag"),
        None
    );
}

#[test]
fn detached_dataset_reads_native_attributes_after_state_drift_and_method_tamper() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const node = d.createElement('p');
                node.setAttribute('id', 'detached-native-dataset');
                node.setAttribute('data-origin', 'js-state');
                node.setAttribute('data-stale', 'js-stale');
                d.body.appendChild(node);
                globalThis.__detachedNativeDatasetNode = node;
                return node.dataset.origin + '|' + node.dataset.stale;
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "js-state|js-stale");

    let handle = element_handle_by_id(&vm, "detached-native-dataset");
    let dom_host = vm.document_runtime.dom_host_mut();
    assert!(dom_host.set_attribute(handle, "data-origin", "native-state"));
    assert!(dom_host.set_attribute(handle, "data-added", "native-only"));
    assert!(dom_host.remove_attribute(handle, "data-stale"));

    let status = vm
        .eval(
            r#"
            (() => {
                const node = globalThis.__detachedNativeDatasetNode;
                node.getAttributeNames = () => ['data-origin', 'data-fake', 'data-stale'];
                node.getAttribute = name => {
                    if (name === 'data-origin') return 'js-tamper';
                    if (name === 'data-added') return null;
                    return 'fake';
                };
                node.hasAttribute = name => name !== 'data-missing';
                const dataset = node.dataset;
                return [
                    Object.prototype.hasOwnProperty.call(node, 'dataset'),
                    typeof Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'dataset')?.get,
                    dataset.origin,
                    dataset.added,
                    dataset.stale === undefined,
                    'stale' in dataset,
                    Object.keys(dataset).sort().join(','),
                    dataset === node.dataset
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed dataset should evaluate");
    assert_eq!(
        status,
        "false|function|native-state|native-only|true|false|added,origin|true"
    );
}

#[test]
fn detached_dataset_writes_native_attributes_with_method_tamper() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const node = d.createElement('p');
                node.setAttribute('id', 'detached-native-dataset-write');
                node.setAttribute('data-origin', 'before-delete');
                d.body.appendChild(node);
                globalThis.__detachedNativeDatasetWriteNode = node;
                return node.dataset.origin;
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "before-delete");

    let status = vm
        .eval(
            r#"
            (() => {
                const node = globalThis.__detachedNativeDatasetWriteNode;
                Object.defineProperty(node, 'setAttribute', {
                    configurable: true,
                    value() { throw new Error('setAttribute should not be called'); }
                });
                Object.defineProperty(node, 'removeAttribute', {
                    configurable: true,
                    value() { throw new Error('removeAttribute should not be called'); }
                });
                const dataset = node.dataset;
                dataset.added = 42;
                delete dataset.origin;
                let invalidName;
                try {
                    dataset['bad-name'] = 'x';
                    invalidName = 'no-throw';
                } catch (error) {
                    invalidName = error.name;
                }
                return [
                    Object.prototype.hasOwnProperty.call(node, 'dataset'),
                    dataset.added,
                    'origin' in dataset,
                    Object.keys(dataset).sort().join(','),
                    invalidName
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed dataset writes should evaluate");
    assert_eq!(status, "false|42|false|added|SyntaxError");

    let handle = element_handle_by_id(&vm, "detached-native-dataset-write");
    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(
        dom_host.get_attribute(handle, "data-added").as_deref(),
        Some("42")
    );
    assert_eq!(dom_host.get_attribute(handle, "data-origin"), None);
    assert_eq!(dom_host.get_attribute(handle, "data-bad-name"), None);
}

#[test]
fn detached_class_list_uses_native_attribute_after_method_tamper() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const node = d.createElement('p');
                node.setAttribute('id', 'detached-native-class-list');
                node.setAttribute('class', 'alpha beta');
                d.body.appendChild(node);
                globalThis.__detachedNativeClassListNode = node;
                return node.classList.value;
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "alpha beta");

    let handle = element_handle_by_id(&vm, "detached-native-class-list");
    let dom_host = vm.document_runtime.dom_host_mut();
    assert!(dom_host.set_attribute(handle, "class", "native-only gamma"));

    let status = vm
        .eval(
            r#"
            (() => {
                const node = globalThis.__detachedNativeClassListNode;
                let getCalled = false;
                let setCalled = false;
                Object.defineProperty(node, 'getAttribute', {
                    configurable: true,
                    value() {
                        getCalled = true;
                        return 'tampered';
                    }
                });
                Object.defineProperty(node, 'setAttribute', {
                    configurable: true,
                    value() {
                        setCalled = true;
                    }
                });
                const list = node.classList;
                const before = [
                    list.value,
                    list.contains('native-only'),
                    list.contains('tampered'),
                    list.length,
                    list.item(1)
                ].join(',');
                list.add('delta');
                list.remove('gamma');
                return [
                    before,
                    list.value,
                    getCalled,
                    setCalled
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed classList should evaluate");
    assert_eq!(
        status,
        "native-only gamma,true,false,2,gamma|native-only delta|false|false"
    );

    let dom_host = vm.document_runtime.dom_host();
    assert_eq!(
        dom_host.get_attribute(handle, "class").as_deref(),
        Some("native-only delta")
    );
}

#[test]
fn detached_clone_reads_native_attributes_after_state_drift() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const node = d.createElement('p');
                node.setAttribute('id', 'detached-native-clone');
                node.setAttribute('data-origin', 'js-state');
                node.setAttributeNS('urn:clone', 'c:flag', 'js-state');
                d.body.appendChild(node);
                globalThis.__detachedNativeCloneNode = node;
                return node.getAttribute('data-origin') + '|' + node.getAttributeNS('urn:clone', 'flag');
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "js-state|js-state");

    let handle = vm
        .document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            let element = node.as_element()?;
            (element.local_name() == "p"
                && element.attribute("id") == Some("detached-native-clone"))
            .then_some(DomHandle::new(index))
        })
        .expect("detached element should have a native handle");
    let dom_host = vm.document_runtime.dom_host_mut();
    assert!(dom_host.set_attribute(handle, "data-origin", "native-state"));
    assert!(dom_host.set_attribute(handle, "data-added", "native-only"));
    assert!(dom_host.set_attribute_ns(
        handle,
        Some("urn:clone"),
        Some("native"),
        "flag",
        "native-state",
    ));

    let status = vm
        .eval(
            r#"
            (() => {
                const clone = globalThis.__detachedNativeCloneNode.cloneNode(false);
                clone.setAttribute('id', 'detached-native-clone-copy');
                const attr = clone.getAttributeNodeNS('urn:clone', 'flag');
                return [
                    clone.getAttribute('data-origin'),
                    clone.getAttribute('data-added'),
                    clone.getAttributeNS('urn:clone', 'flag'),
                    clone.hasAttribute('data-added'),
                    clone.getAttributeNames().includes('data-added'),
                    attr && attr.name,
                    attr && attr.prefix,
                    attr && attr.namespaceURI,
                    attr && attr.localName,
                    attr && attr.value
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed clone should evaluate");
    assert_eq!(
        status,
        "native-state|native-only|native-state|true|true|c:flag|c|urn:clone|flag|native-state"
    );

    let clone_handle = element_handle_by_id(&vm, "detached-native-clone-copy");
    let clone = vm
        .document_runtime
        .dom_host()
        .node(clone_handle)
        .and_then(|node| node.as_element())
        .expect("cloned element should remain native-backed");
    let plain_namespaced_name_count = clone
        .attributes()
        .iter()
        .filter(|attribute| attribute.namespace().is_empty() && attribute.name() == "c:flag")
        .count();
    let namespaced_flag_count = clone
        .attributes()
        .iter()
        .filter(|attribute| {
            attribute.namespace() == "urn:clone" && attribute.local_name() == "flag"
        })
        .count();
    assert_eq!(
        plain_namespaced_name_count, 0,
        "namespaced clone must not also create a plain c:flag attribute"
    );
    assert_eq!(
        namespaced_flag_count, 1,
        "namespaced clone should create exactly one urn:clone flag attribute"
    );
}

#[test]
fn detached_character_data_mutations_sync_to_native_handle() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const text = d.createTextNode('alpha');
            d.body.appendChild(text);
            const observer = new MutationObserver(() => {});
            observer.observe(text, { characterData: true, characterDataOldValue: true });
            text.appendData('-beta');
            text.deleteData(0, 6);
            text.insertData(0, 'native-');
            text.replaceData(7, 4, 'data');
            return [
                text.data,
                observer.takeRecords()
                    .map((record) => `${record.type}:${record.oldValue}:${record.target === text}`)
                    .join(',')
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "native-data|characterData:alpha:true,characterData:alpha-beta:true,characterData:beta:true,characterData:native-beta:true"
    );

    let has_native_text = vm
        .document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .any(|node| node.node_value() == Some("native-data"));
    assert!(
        has_native_text,
        "detached text data should sync to native DOM"
    );
}

#[test]
fn detached_character_data_setters_write_native_handle() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const text = d.createTextNode('initial-text');
            const comment = d.createComment('initial-comment');
            const pi = d.createProcessingInstruction('target', 'initial-pi');
            d.body.append(text, comment, pi);
            text.data = 'data-setter-native';
            comment.nodeValue = 'node-value-native';
            pi.data = 'pi-data-native';
            return [text.data, comment.data, pi.nodeValue].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "data-setter-native|node-value-native|pi-data-native"
    );

    for value in ["data-setter-native", "node-value-native", "pi-data-native"] {
        let _ = text_handle_by_value(&vm, value);
    }
}

#[test]
fn detached_character_data_reads_prefer_native_handle() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const text = d.createTextNode('js-state');
                d.body.appendChild(text);
                globalThis.__detachedNativeReadText = text;
                return text.data;
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "js-state");

    let handle = vm
        .document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            (node.node_value() == Some("js-state")).then_some(DomHandle::new(index))
        })
        .expect("detached text should have a native handle");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .set_text_content(handle, "native-state")
    );

    let status = vm
        .eval(
            r#"
            (() => {
                const text = globalThis.__detachedNativeReadText;
                return [
                    text.data,
                    text.length,
                    text.substringData(7, 5)
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed character data reads should evaluate");
    assert_eq!(status, "native-state|12|state");
}

#[test]
fn detached_tree_navigation_reads_prefer_native_handle_order() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const parent = d.createElement('div');
                const a = d.createElement('section');
                const b = d.createElement('article');
                parent.id = 'native-tree-parent';
                a.id = 'native-tree-a';
                b.id = 'native-tree-b';
                parent.appendChild(a);
                parent.appendChild(b);
                d.body.appendChild(parent);
                globalThis.__nativeTreeParent = parent;
                globalThis.__nativeTreeA = a;
                globalThis.__nativeTreeB = b;
                return parent.firstChild === a && parent.lastChild === b ? 'ok' : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let parent = element_handle_by_id(&vm, "native-tree-parent");
    let a = element_handle_by_id(&vm, "native-tree-a");
    {
        let dom_host = vm.document_runtime.dom_host_mut();
        assert!(dom_host.remove_child(parent, a));
        assert!(dom_host.insert_before(parent, a, None));
    }

    let status = vm
        .eval(
            r#"
            (() => {
                const parent = globalThis.__nativeTreeParent;
                const a = globalThis.__nativeTreeA;
                const b = globalThis.__nativeTreeB;
                const childNodes = parent.childNodes;
                const children = parent.children;
                return [
                    parent.firstChild === b,
                    parent.lastChild === a,
                    childNodes.length,
                    childNodes[0] === b,
                    childNodes.item(1) === a,
                    children.length,
                    children[0] === b,
                    children.item(1) === a,
                    b.nextSibling === a,
                    a.previousSibling === b,
                    b.previousSibling === null,
                    a.nextSibling === null,
                    a.parentNode === parent,
                    a.parentElement === parent,
                    parent.contains(a)
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed tree reads should evaluate");
    assert_eq!(
        status,
        "true|true|2|true|true|2|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn detached_tree_navigation_reads_native_detachment() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const parent = d.createElement('div');
                const child = d.createElement('span');
                parent.id = 'native-detach-parent';
                child.id = 'native-detach-child';
                parent.appendChild(child);
                d.body.appendChild(parent);
                globalThis.__nativeDetachParent = parent;
                globalThis.__nativeDetachChild = child;
                return parent.hasChildNodes() && child.parentNode === parent ? 'ok' : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let parent = element_handle_by_id(&vm, "native-detach-parent");
    let child = element_handle_by_id(&vm, "native-detach-child");
    assert!(
        vm.document_runtime
            .dom_host_mut()
            .remove_child(parent, child)
    );

    let status = vm
        .eval(
            r#"
            (() => {
                const parent = globalThis.__nativeDetachParent;
                const child = globalThis.__nativeDetachChild;
                const childNodes = parent.childNodes;
                return [
                    parent.hasChildNodes(),
                    parent.firstChild === null,
                    parent.lastChild === null,
                    childNodes.length,
                    child.parentNode === null,
                    child.parentElement === null,
                    child.previousSibling === null,
                    child.nextSibling === null,
                    child.isConnected,
                    parent.contains(child)
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed detached reads should evaluate");
    assert_eq!(status, "false|true|true|0|true|true|true|true|false|false");
}

#[test]
fn detached_child_reads_ignore_tampered_children_projection() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('section');
            const real = d.createElement('span');
            const fake = d.createElement('fake-node');
            real.id = 'real';
            fake.id = 'fake';
            parent.appendChild(real);
            const projected = parent.childNodes;
            projected[0] = fake;
            projected.length = 1;
            return [
                parent.firstChild === real,
                parent.lastChild === real,
                parent.hasChildNodes(),
                parent.children.length,
                parent.children[0] === real,
                real.previousSibling === null,
                real.nextSibling === null,
                fake.parentNode === null
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|1|true|true|true|true");
}

#[test]
fn detached_remove_child_uses_native_parent_after_native_move() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const oldParent = d.createElement('div');
                const newParent = d.createElement('section');
                const child = d.createElement('span');
                oldParent.id = 'native-remove-old-parent';
                newParent.id = 'native-remove-new-parent';
                child.id = 'native-remove-child';
                oldParent.appendChild(child);
                d.body.appendChild(oldParent);
                d.body.appendChild(newParent);
                globalThis.__nativeRemoveNewParent = newParent;
                globalThis.__nativeRemoveChild = child;
                return child.parentNode === oldParent ? 'ok' : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let old_parent = element_handle_by_id(&vm, "native-remove-old-parent");
    let new_parent = element_handle_by_id(&vm, "native-remove-new-parent");
    let child = element_handle_by_id(&vm, "native-remove-child");
    {
        let dom_host = vm.document_runtime.dom_host_mut();
        assert!(dom_host.remove_child(old_parent, child));
        assert!(dom_host.insert_before(new_parent, child, None));
    }

    let status = vm
        .eval(
            r#"
            (() => {
                const newParent = globalThis.__nativeRemoveNewParent;
                const child = globalThis.__nativeRemoveChild;
                const removed = newParent.removeChild(child);
                return [
                    removed === child,
                    newParent.childNodes.length,
                    child.parentNode === null,
                    child.parentElement === null,
                    newParent.contains(child)
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed removeChild should evaluate");
    assert_eq!(status, "true|0|true|true|false");
}

#[test]
fn detached_remove_child_uses_native_parent_after_parent_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const a = d.createElement('a');
            const b = d.createElement('b');
            a.id = 'a';
            b.id = 'b';
            parent.append(a, b);
            d.body.appendChild(parent);
            Object.defineProperty(b, 'parentNode', {
                value: null,
                configurable: true
            });
            const removed = parent.removeChild(b);
            return [
                removed === b,
                parent.childNodes.length,
                parent.lastChild === a,
                b.parentNode === null,
                b.isConnected,
                parent.contains(b)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|1|true|true|false|false");
}

#[test]
fn detached_replace_children_removes_native_children_missing_from_js_state() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const parent = d.createElement('div');
                const other = d.createElement('section');
                const stale = d.createElement('span');
                const nativeOnly = d.createElement('em');
                parent.id = 'native-replace-children-parent';
                other.id = 'native-replace-children-other';
                stale.id = 'native-replace-children-stale';
                nativeOnly.id = 'native-replace-children-native-only';
                parent.appendChild(stale);
                other.appendChild(nativeOnly);
                d.body.appendChild(parent);
                d.body.appendChild(other);
                globalThis.__nativeReplaceChildrenParent = parent;
                globalThis.__nativeReplaceChildrenOther = other;
                globalThis.__nativeReplaceChildrenNativeOnly = nativeOnly;
                return parent.childNodes.length === 1 && other.childNodes.length === 1 ? 'ok' : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let parent = element_handle_by_id(&vm, "native-replace-children-parent");
    let other = element_handle_by_id(&vm, "native-replace-children-other");
    let native_only = element_handle_by_id(&vm, "native-replace-children-native-only");
    {
        let dom_host = vm.document_runtime.dom_host_mut();
        assert!(dom_host.remove_child(other, native_only));
        assert!(dom_host.insert_before(parent, native_only, None));
    }

    let status = vm
        .eval(
            r#"
            (() => {
                const parent = globalThis.__nativeReplaceChildrenParent;
                const other = globalThis.__nativeReplaceChildrenOther;
                const nativeOnly = globalThis.__nativeReplaceChildrenNativeOnly;
                const replacement = parent.ownerDocument.createElement('strong');
                replacement.id = 'native-replace-children-replacement';
                parent.replaceChildren(replacement);
                return [
                    parent.childNodes.length,
                    parent.firstChild === replacement,
                    nativeOnly.parentNode === null,
                    nativeOnly.isConnected,
                    other.childNodes.length
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed replaceChildren should evaluate");
    assert_eq!(status, "1|true|true|false|0");
}

#[test]
fn detached_insert_before_uses_native_reference_parent_after_parent_getter_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const a = d.createElement('a');
            const b = d.createElement('b');
            const c = d.createElement('c');
            parent.appendChild(a);
            parent.appendChild(b);
            d.body.appendChild(parent);
            Object.defineProperty(b, 'parentNode', {
                value: null,
                configurable: true
            });
            const inserted = parent.insertBefore(c, b);
            return [
                inserted === c,
                parent.firstChild === a,
                a.nextSibling === c,
                c.nextSibling === b,
                b.previousSibling === c,
                parent.children.length
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true|true|3");
}

#[test]
fn detached_before_after_use_native_parent_and_sibling_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const a = d.createElement('a');
            const b = d.createElement('b');
            const c = d.createElement('c');
            const before = d.createElement('before-node');
            const after = d.createElement('after-node');
            a.id = 'a';
            b.id = 'b';
            c.id = 'c';
            before.id = 'before';
            after.id = 'after';
            parent.append(a, b, c);
            d.body.appendChild(parent);
            Object.defineProperty(b, 'parentNode', {
                value: null,
                configurable: true
            });
            Object.defineProperty(b, 'nextSibling', {
                value: null,
                configurable: true
            });
            b.before(before);
            b.after(after);
            return [
                parent.childNodes.length,
                Array.from(parent.childNodes).map(node => node.id).join(','),
                before.parentNode === parent,
                after.parentNode === parent,
                after.nextSibling === c
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "5|a,before,b,after,c|true|true|true");
}

#[test]
fn detached_prepend_uses_native_first_child_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const a = d.createElement('a');
            const b = d.createElement('b');
            const inserted = d.createElement('inserted-node');
            a.id = 'a';
            b.id = 'b';
            inserted.id = 'inserted';
            parent.append(a, b);
            d.body.appendChild(parent);
            Object.defineProperty(parent, 'firstChild', {
                value: null,
                configurable: true
            });
            parent.prepend(inserted);
            delete parent.firstChild;
            const nodes = parent.childNodes;
            return [
                nodes[0] === inserted,
                Array.from(nodes).map(node => node.id).join(','),
                inserted.nextSibling === a
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|inserted,a,b|true");
}

#[test]
fn detached_append_variadic_builds_fragment_without_public_append_child() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const child = d.createElement('span');
            child.id = 'child';
            d.body.appendChild(parent);
            const fragmentProto = Object.getPrototypeOf(d.createDocumentFragment());
            Object.defineProperty(fragmentProto, 'appendChild', {
                value() {
                    throw new Error('fragment appendChild should not be called');
                },
                configurable: true
            });
            parent.append('text-', child, '-tail');
            return [
                parent.childNodes.length,
                parent.childNodes[0].data,
                parent.childNodes[1] === child,
                parent.childNodes[2].data,
                child.parentNode === parent
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "3|text-|true|-tail|true");
}

#[test]
fn detached_append_variadic_builds_fragment_without_public_create_document_fragment() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            d.body.appendChild(parent);
            Object.defineProperty(d, 'createDocumentFragment', {
                value() {
                    throw new Error('createDocumentFragment should not be called');
                },
                configurable: true
            });
            parent.append('text');
            return [
                parent.childNodes.length,
                parent.firstChild.data,
                parent.firstChild.parentNode === parent
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "1|text|true");
}

#[test]
fn detached_variadic_mutations_use_native_helpers_after_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const head = d.createElement('head-node');
            const anchor = d.createElement('anchor-node');
            const tail = d.createElement('tail-node');
            const before = d.createElement('before-node');
            const after = d.createElement('after-node');
            const replacement = d.createElement('replacement-node');
            head.id = 'head';
            anchor.id = 'anchor';
            tail.id = 'tail';
            before.id = 'before';
            after.id = 'after';
            replacement.id = 'replacement';
            d.body.appendChild(parent);
            for (const name of ['appendChild', 'insertBefore', 'removeChild']) {
                Object.defineProperty(parent, name, {
                    value() {
                        throw new Error(name + ' should not be called');
                    },
                    configurable: true
                });
            }
            parent.append('tail-text');
            parent.prepend(head);
            const firstPass = Array.from(parent.childNodes)
                .map(node => node.id || node.data)
                .join(',');
            parent.replaceChildren(anchor, tail);
            tail.before(before);
            anchor.after(after);
            anchor.replaceWith(replacement);
            return [
                firstPass,
                Array.from(parent.childNodes).map(node => node.id).join(','),
                replacement.parentNode === parent,
                anchor.parentNode === null,
                after.previousSibling === replacement,
                before.nextSibling === tail
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "head,tail-text|replacement,after,before,tail|true|true|true|true"
    );
}

#[test]
fn detached_inner_html_setter_uses_native_replace_after_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const old = d.createElement('old-node');
            old.id = 'old';
            parent.appendChild(old);
            d.body.appendChild(parent);
            Object.defineProperty(parent, 'replaceChildren', {
                value() {
                    throw new Error('replaceChildren should not be called');
                },
                configurable: true
            });
            Object.defineProperty(parent, 'childNodes', {
                get() {
                    throw new Error('childNodes should not be read');
                },
                configurable: true
            });
            parent.innerHTML = '<span id="first">alpha</span><em id="second"></em>';
            const first = parent.firstChild;
            const second = parent.lastChild;
            return [
                first && first.id,
                first && first.firstChild && first.firstChild.data,
                second && second.id,
                first && first.parentNode === parent,
                second && second.previousSibling === first,
                old.parentNode === null
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "first|alpha|second|true|true|true");
}

#[test]
fn detached_shadow_root_inner_html_serializes_native_attributes_after_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const host = d.createElement('div');
            d.body.appendChild(host);
            const root = host.attachShadow({ mode: 'open' });
            root.innerHTML = '<span id="shadow-attr" data-native="ok"></span>';
            const child = root.firstChild;
            let tampered = false;
            Object.defineProperty(child, 'localName', {
                get() {
                    tampered = true;
                    return 'section';
                },
                configurable: true
            });
            Object.defineProperty(child, 'tagName', {
                get() {
                    tampered = true;
                    return 'section';
                },
                configurable: true
            });
            Object.defineProperty(child, 'getAttributeNames', {
                value() {
                    tampered = true;
                    return ['data-native'];
                },
                configurable: true
            });
            Object.defineProperty(child, 'getAttribute', {
                value() {
                    tampered = true;
                    return 'wrong';
                },
                configurable: true
            });
            const html = root.innerHTML;
            return [
                html.startsWith('<span '),
                html.includes('id="shadow-attr"'),
                html.includes('data-native="ok"'),
                html.includes('<section'),
                html.includes('wrong'),
                tampered
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|false|false|false");
}

#[test]
fn detached_document_stylesheets_ignore_unloaded_link_after_get_attribute_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const link = d.createElement('link');
            link.id = 'native-sheet';
            link.setAttribute('rel', 'alternate stylesheet');
            link.setAttribute('title', 'native');
            d.head.appendChild(link);
            let tampered = false;
            Object.defineProperty(link, 'getAttribute', {
                value() {
                    tampered = true;
                    return '';
                },
                configurable: true
            });
            return [
                Array.from(d.styleSheets).map(sheet => sheet.ownerNode.id).join(','),
                tampered
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "|false");
}

#[test]
fn detached_document_collections_use_native_metadata_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const img = d.createElement('img');
            const embed = d.createElement('embed');
            const link = d.createElement('a');
            const area = d.createElement('area');
            const form = d.createElement('form');
            const script = d.createElement('script');
            const sheet = d.createElement('link');
            img.id = 'native-img';
            embed.id = 'native-embed';
            link.id = 'native-link';
            link.href = '/link';
            link.setAttribute('name', 'native-anchor');
            area.id = 'native-area';
            area.href = '/area';
            form.id = 'native-form';
            script.id = 'native-script';
            sheet.id = 'native-sheet';
            sheet.setAttribute('rel', 'alternate stylesheet');
            sheet.setAttribute('title', 'native');
            d.body.append(img, embed, link, area, form, script, sheet);

            let hasAttributeCalled = false;
            let getAttributeCalled = false;
            for (const node of [img, embed, link, area, form, script, sheet]) {
                Object.defineProperty(node, 'localName', {
                    configurable: true,
                    get() {
                        return 'tampered';
                    }
                });
                node.hasAttribute = () => {
                    hasAttributeCalled = true;
                    return false;
                };
                node.getAttribute = () => {
                    getAttributeCalled = true;
                    return null;
                };
            }

            return [
                d.images.item(0) === img,
                d.embeds.item(0) === embed,
                d.links.length,
                d.links.item(0) === link,
                d.links.item(1) === area,
                d.anchors.item(0) === link,
                d.forms.item(0) === form,
                d.scripts.item(0) === script,
                Array.from(d.styleSheets).map(sheet => sheet.ownerNode.id).join(','),
                hasAttributeCalled,
                getAttributeCalled
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|2|true|true|true|true|true||false|false");
}

#[test]
fn detached_focus_uses_native_local_name_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const input = d.createElement('input');
            d.body.appendChild(input);
            let tampered = false;
            Object.defineProperty(input, 'localName', {
                configurable: true,
                get() {
                    tampered = true;
                    return 'div';
                }
            });
            input.focus();
            return [d.activeElement === input, tampered].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|false");
}

#[test]
fn detached_shadow_focus_uses_private_host_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const host = d.createElement('div');
            d.body.appendChild(host);
            const root = host.attachShadow({ mode: 'open' });
            const input = d.createElement('input');
            root.appendChild(input);

            let nodeNameRead = false;
            let hostRead = false;
            Object.defineProperty(root, 'nodeName', {
                configurable: true,
                get() {
                    nodeNameRead = true;
                    return '#tampered';
                }
            });
            Object.defineProperty(root, 'host', {
                configurable: true,
                get() {
                    hostRead = true;
                    return null;
                }
            });

            input.focus();
            return [
                d.activeElement === host,
                root.activeElement === input,
                nodeNameRead,
                hostRead
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|false|false");
}

#[test]
fn detached_shadow_root_stylesheets_use_native_metadata_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const host = d.createElement('div');
            d.body.appendChild(host);
            const root = host.attachShadow({ mode: 'open' });
            const sheet = d.createElement('link');
            sheet.id = 'shadow-sheet';
            sheet.setAttribute('rel', 'alternate stylesheet');
            root.appendChild(sheet);

            let tampered = false;
            Object.defineProperty(sheet, 'localName', {
                get() {
                    tampered = true;
                    return 'section';
                },
                configurable: true
            });
            sheet.getAttribute = () => {
                tampered = true;
                return null;
            };

            return [
                Array.from(root.styleSheets).map(node => node.id).join(','),
                tampered
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "shadow-sheet|false");
}

#[test]
fn detached_fragment_clone_uses_native_builder_and_insert_after_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const fragment = d.createDocumentFragment();
            const child = d.createElement('span');
            child.id = 'child';
            fragment.appendChild(child);
            const fragmentProto = Object.getPrototypeOf(fragment);
            Object.defineProperty(d, 'createDocumentFragment', {
                value() {
                    throw new Error('createDocumentFragment should not be called');
                },
                configurable: true
            });
            Object.defineProperty(fragmentProto, 'appendChild', {
                value() {
                    throw new Error('fragment appendChild should not be called');
                },
                configurable: true
            });
            const cloned = fragment.cloneNode(true);
            return [
                cloned !== fragment,
                cloned.childNodes.length,
                cloned.firstChild !== child,
                cloned.firstChild.id,
                cloned.firstChild.parentNode === cloned,
                fragment.firstChild === child
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|1|true|child|true|true");
}

#[test]
fn detached_character_node_clone_uses_native_builders_after_document_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const text = d.createTextNode('text-data');
            const comment = d.createComment('comment-data');
            const pi = d.createProcessingInstruction('pi-target', 'pi-data');
            for (const name of ['createTextNode', 'createComment', 'createProcessingInstruction']) {
                Object.defineProperty(d, name, {
                    value() {
                        throw new Error(name + ' should not be called');
                    },
                    configurable: true
                });
            }
            const textClone = text.cloneNode();
            const commentClone = comment.cloneNode();
            const piClone = pi.cloneNode();
            return [
                textClone !== text,
                textClone.data,
                textClone.ownerDocument === d,
                commentClone !== comment,
                commentClone.data,
                piClone !== pi,
                piClone.target,
                piClone.data
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "true|text-data|true|true|comment-data|true|pi-target|pi-data"
    );
}

#[test]
fn detached_element_deep_clone_uses_native_builder_and_insert_after_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const child = d.createElement('span');
            parent.id = 'parent';
            child.id = 'child';
            parent.setAttribute('data-x', '1');
            parent.appendChild(child);
            const elementProto = Object.getPrototypeOf(parent);
            for (const name of ['createElement', 'createElementNS']) {
                Object.defineProperty(d, name, {
                    value() {
                        throw new Error(name + ' should not be called');
                    },
                    configurable: true
                });
            }
            Object.defineProperty(elementProto, 'appendChild', {
                value() {
                    throw new Error('appendChild should not be called');
                },
                configurable: true
            });
            const cloned = parent.cloneNode(true);
            return [
                cloned !== parent,
                cloned.localName,
                cloned.getAttribute('data-x'),
                cloned.childNodes.length,
                cloned.firstChild !== child,
                cloned.firstChild.id,
                cloned.firstChild.parentNode === cloned,
                parent.firstChild === child
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|div|1|1|true|child|true|true");
}

#[test]
fn import_xhtml_element_into_html_document_preserves_html_tag_name_semantics() {
    let status = eval(
        r#"
        (() => {
            const HTML_NS = "http://www.w3.org/1999/xhtml";
            const xmlElement = document.implementation
                .createDocument(HTML_NS, "foo:div", null)
                .documentElement;
            const imported = document.importNode(xmlElement, true);
            return [
                xmlElement.tagName,
                imported.tagName,
                imported.localName,
                imported.prefix,
                imported.namespaceURI
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "foo:div|FOO:DIV|div|foo|http://www.w3.org/1999/xhtml"
    );
}

#[test]
fn detached_xml_document_clone_uses_native_insert_after_replace_children_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createDocument('urn:test', 'root', null);
            const child = d.createElementNS('urn:test', 'leaf');
            child.setAttribute('data-x', '1');
            d.documentElement.appendChild(child);
            Object.defineProperty(Object.getPrototypeOf(d), 'replaceChildren', {
                value() {
                    throw new Error('replaceChildren should not be called');
                },
                configurable: true
            });
            const cloned = d.cloneNode(true);
            return [
                cloned !== d,
                cloned.documentElement.nodeName,
                cloned.documentElement.firstChild.nodeName,
                cloned.documentElement.firstChild.getAttribute('data-x'),
                cloned.documentElement.firstChild.parentNode === cloned.documentElement
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|root|leaf|1|true");
}

#[test]
fn detached_html_document_clone_uses_native_root_metadata_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            let reads = 0;
            Object.defineProperty(d.documentElement, 'localName', {
                get() { reads++; return 'svg'; },
                configurable: true
            });
            const cloned = d.cloneNode(false);
            const deep = d.cloneNode(true);
            return [
                cloned !== d,
                cloned.contentType,
                cloned.childNodes.length,
                cloned.documentElement === null,
                cloned.head === null,
                cloned.body === null,
                deep.documentElement.localName,
                deep.head !== null,
                deep.body !== null,
                reads
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|text/html|0|true|true|true|html|true|true|0");
}

#[test]
fn detached_document_clones_preserve_the_full_native_quirks_mode() {
    let (status, vm) = eval_with_vm(
        r#"(() => {
            const source = new DOMParser().parseFromString(
                '<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Transitional//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd"><p>text</p>',
                'text/html'
            );
            for (const deep of [false, true]) {
                const clone = source.cloneNode(deep);
                const marker = clone.createElement('section');
                marker.id = 'limited-quirks-clone-' + deep;
                if (deep) clone.body.append(marker);
                else clone.append(marker);
            }
            return source.compatMode;
        })()"#,
    );
    assert_eq!(status, "CSS1Compat");
    for deep in [false, true] {
        let marker = element_handle_by_id(&vm, &format!("limited-quirks-clone-{deep}"));
        let host = vm.document_runtime.dom_host();
        let owner = host.node(marker).unwrap().owner_document().unwrap();
        assert_eq!(
            host.document_quirks_mode_for_handle(owner),
            Some(selectors::matching::QuirksMode::LimitedQuirks),
            "cloning must not collapse LimitedQuirks into NoQuirks"
        );
    }
}

#[test]
fn detached_adopt_node_uses_native_detach_after_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d1 = document.implementation.createHTMLDocument('');
            const d2 = document.implementation.createHTMLDocument('');
            const parent = d1.createElement('section');
            const child = d1.createElement('span');
            parent.appendChild(child);
            d1.body.appendChild(parent);
            Object.defineProperty(d1.body, 'removeChild', {
                value() {
                    throw new Error('removeChild should not be called');
                },
                configurable: true
            });
            Object.defineProperty(d2, 'createDocumentFragment', {
                value() {
                    throw new Error('createDocumentFragment should not be called');
                },
                configurable: true
            });
            const returned = d2.adoptNode(parent);
            return [
                returned === parent,
                parent.parentNode === null,
                parent.ownerDocument === d2,
                child.ownerDocument === d2,
                parent.firstChild === child,
                child.parentNode === parent,
                d1.body.contains(parent)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true|true|true|false");
}

#[test]
fn detached_adopt_live_node_uses_native_detach_without_fragment_bridge() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const liveHost = document.createElement('main');
            const parent = document.createElement('section');
            const child = document.createElement('span');
            const text = document.createTextNode('native text');
            child.id = 'child';
            child.appendChild(text);
            parent.appendChild(child);
            liveHost.appendChild(parent);
            Object.defineProperty(parent, 'parentNode', {
                get() {
                    throw new Error('parentNode should not be read');
                },
                configurable: true
            });
            Object.defineProperty(parent, 'childNodes', {
                get() {
                    throw new Error('childNodes should not be read');
                },
                configurable: true
            });
            Object.defineProperty(text, 'data', {
                get() {
                    throw new Error('text data should not be read');
                },
                configurable: true
            });
            Object.defineProperty(text, 'nodeValue', {
                get() {
                    throw new Error('text nodeValue should not be read');
                },
                configurable: true
            });
            Object.defineProperty(liveHost, 'removeChild', {
                value() {
                    throw new Error('live removeChild should not be called');
                },
                configurable: true
            });
            Object.defineProperty(d, 'createDocumentFragment', {
                value() {
                    throw new Error('createDocumentFragment should not be called');
                },
                configurable: true
            });
            const returned = d.adoptNode(parent);
            delete parent.parentNode;
            delete text.data;
            delete text.nodeValue;
            return [
                returned === parent,
                parent.parentNode === null,
                parent.ownerDocument === d,
                child.ownerDocument === d,
                parent.firstChild === child,
                child.parentNode === parent,
                child.firstChild === text,
                text.data,
                liveHost.contains(parent)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "true|true|true|true|true|true|true|native text|false"
    );
}

#[test]
fn detached_adopt_live_native_subtree_uses_native_child_and_data_sources() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const liveHost = document.createElement('main');
            const parent = document.createElement('section');
            const child = document.createElement('span');
            const text = document.createTextNode('native text');
            const pi = document.createProcessingInstruction('xml-stylesheet', "href='native.css'");
            parent.id = 'native-live-adopt-parent';
            child.id = 'native-live-adopt-child';
            child.appendChild(text);
            parent.appendChild(child);
            parent.appendChild(pi);
            liveHost.appendChild(parent);
            Object.defineProperty(parent, 'childNodes', {
                get() {
                    throw new Error('parent childNodes should not be read');
                },
                configurable: true
            });
            for (const node of [text, pi]) {
                Object.defineProperty(node, 'data', {
                    get() {
                        throw new Error('character data should not be read');
                    },
                    configurable: true
                });
                Object.defineProperty(node, 'nodeValue', {
                    get() {
                        throw new Error('nodeValue should not be read');
                    },
                    configurable: true
                });
            }
            const returned = d.adoptNode(parent);
            delete parent.childNodes;
            delete text.data;
            delete text.nodeValue;
            delete pi.data;
            delete pi.nodeValue;
            return [
                returned === parent,
                parent.parentNode === null,
                parent.firstChild === child,
                child.parentNode === parent,
                child.firstChild === text,
                text.data,
                parent.lastChild === pi,
                pi.target,
                pi.data,
                liveHost.contains(parent)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "true|true|true|true|true|native text|true|xml-stylesheet|href='native.css'|false"
    );

    let parent = element_handle_by_id(&vm, "native-live-adopt-parent");
    let child = element_handle_by_id(&vm, "native-live-adopt-child");
    let dom = vm.document_runtime.dom_host().dom();
    assert_eq!(dom.parent_node(child), Some(parent));
    assert_eq!(dom.child_ids(parent).count(), 2);
}

#[test]
fn detached_insert_live_node_uses_native_detach_after_parent_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const target = d.createElement('div');
            const liveHost = document.createElement('main');
            const liveNode = document.createElement('section');
            const liveChild = document.createElement('span');
            liveChild.id = 'live-child';
            liveNode.appendChild(liveChild);
            liveHost.appendChild(liveNode);
            Object.defineProperty(liveNode, 'parentNode', {
                get() {
                    throw new Error('parentNode should not be read');
                },
                configurable: true
            });
            Object.defineProperty(liveHost, 'removeChild', {
                value() {
                    throw new Error('live removeChild should not be called');
                },
                configurable: true
            });
            target.appendChild(liveNode);
            return [
                liveNode.parentNode === target,
                liveNode.ownerDocument === d,
                liveChild.ownerDocument === d,
                liveNode.firstChild === liveChild,
                liveChild.parentNode === liveNode,
                liveHost.contains(liveNode)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true|true|false");
}

#[test]
fn detached_replace_with_uses_native_parent_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const a = d.createElement('a');
            const b = d.createElement('b');
            const c = d.createElement('c');
            const replacement = d.createElement('replacement-node');
            a.id = 'a';
            b.id = 'b';
            c.id = 'c';
            replacement.id = 'replacement';
            parent.append(a, b, c);
            d.body.appendChild(parent);
            Object.defineProperty(b, 'parentNode', {
                value: null,
                configurable: true
            });
            b.replaceWith(replacement);
            return [
                parent.childNodes.length,
                Array.from(parent.childNodes).map(node => node.id).join(','),
                b.parentNode === null,
                b.isConnected,
                replacement.parentNode === parent
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "3|a,replacement,c|true|false|true");
}

#[test]
fn detached_replace_child_uses_native_old_child_parent_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('div');
            const a = d.createElement('a');
            const b = d.createElement('b');
            const c = d.createElement('c');
            const replacement = d.createElement('replacement-node');
            a.id = 'a';
            b.id = 'b';
            c.id = 'c';
            replacement.id = 'replacement';
            parent.append(a, b, c);
            d.body.appendChild(parent);
            Object.defineProperty(b, 'parentNode', {
                value: null,
                configurable: true
            });
            const returned = parent.replaceChild(replacement, b);
            return [
                returned === b,
                parent.childNodes.length,
                Array.from(parent.childNodes).map(node => node.id).join(','),
                b.parentNode === null,
                b.isConnected,
                replacement.parentNode === parent
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|3|a,replacement,c|true|false|true");
}

#[test]
fn detached_document_head_body_reads_follow_native_tree_after_native_detach() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                d.documentElement.id = 'native-document-root';
                d.head.id = 'native-document-head';
                d.body.id = 'native-document-body';
                globalThis.__nativeDocumentStateDoc = d;
                return [
                    d.documentElement.id,
                    d.head.id,
                    d.body.id
                ].join('|');
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(
        setup,
        "native-document-root|native-document-head|native-document-body"
    );

    let html = element_handle_by_id(&vm, "native-document-root");
    let head = element_handle_by_id(&vm, "native-document-head");
    let document = vm
        .document_runtime
        .dom_host()
        .dom()
        .parent_node(html)
        .expect("detached html should have document parent");
    assert!(vm.document_runtime.dom_host_mut().remove_child(html, head));

    let after_head_detach = vm
        .eval(
            r#"
            (() => {
                const d = globalThis.__nativeDocumentStateDoc;
                return [
                    d.documentElement.id,
                    d.head === null,
                    d.body.id
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed document head/body reads should evaluate");
    assert_eq!(
        after_head_detach,
        "native-document-root|true|native-document-body"
    );

    assert!(
        vm.document_runtime
            .dom_host_mut()
            .remove_child(document, html)
    );
    let after_root_detach = vm
        .eval(
            r#"
            (() => {
                const d = globalThis.__nativeDocumentStateDoc;
                return [
                    d.documentElement === null,
                    d.head === null,
                    d.body === null
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed document root reads should evaluate");
    assert_eq!(after_root_detach, "true|true|true");
}

#[test]
fn detached_document_head_body_ignore_tampered_node_metadata() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const html = d.documentElement;
            const head = d.head;
            const body = d.body;
            const reads = [];
            for (const [node, label] of [[html, 'html'], [head, 'head'], [body, 'body']]) {
                Object.defineProperty(node, 'nodeType', {
                    configurable: true,
                    get() {
                        reads.push(`${label}:nodeType`);
                        return Node.COMMENT_NODE;
                    }
                });
                Object.defineProperty(node, 'localName', {
                    configurable: true,
                    get() {
                        reads.push(`${label}:localName`);
                        return 'tampered';
                    }
                });
            }
            return [
                d.documentElement === html,
                d.head === head,
                d.body === body,
                reads.join(',')
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|");
}

#[test]
fn detached_text_content_and_equality_ignore_tampered_child_nodes() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const left = d.createElement('section');
            const right = d.createElement('section');
            left.appendChild(d.createTextNode('native'));
            right.appendChild(d.createTextNode('native'));
            d.body.appendChild(left);
            d.body.appendChild(right);
            const fake = [];
            let leftNodeNameRead = false;
            let rightNodeNameRead = false;
            Object.defineProperty(left, 'childNodes', {
                value: fake,
                configurable: true
            });
            Object.defineProperty(right, 'childNodes', {
                value: fake,
                configurable: true
            });
            Object.defineProperty(left, 'nodeName', {
                get() {
                    leftNodeNameRead = true;
                    return 'tampered-left';
                },
                configurable: true
            });
            Object.defineProperty(right, 'nodeName', {
                get() {
                    rightNodeNameRead = true;
                    return 'tampered-right';
                },
                configurable: true
            });
            return [
                left.textContent,
                right.textContent,
                left.isEqualNode(right),
                leftNodeNameRead,
                rightNodeNameRead
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "native|native|true|false|false");
}

#[test]
fn detached_html_fragment_entrypoints_keep_template_content_without_declarative_shadow() {
    let status = eval(
        r#"
        (() => {
            const content = '<div class="wrapper"><div class="host">' +
                '<template shadowrootmode="open"><span class="content">Content</span></template>' +
                '</div></div>';
            const d = document.implementation.createHTMLDocument('');
            d.body.innerHTML = content;
            const innerHost = d.body.querySelector('.host');
            const innerTemplate = innerHost.querySelector('template');

            const written = document.implementation.createHTMLDocument('');
            written.write('<div id="written">' + content + '</div>');
            const writtenHost = written.getElementById('written').querySelector('.host');
            const writtenTemplate = writtenHost.querySelector('template');

            const rangeDoc = document.implementation.createHTMLDocument('');
            const range = rangeDoc.createRange();
            range.selectNode(rangeDoc.body);
            const fragment = range.createContextualFragment(content);
            const fragmentHost = fragment.querySelector('.host');
            const fragmentTemplate = fragmentHost.querySelector('template');

            return [
                !innerHost.shadowRoot,
                innerTemplate.getAttribute('shadowrootmode'),
                innerTemplate.content.querySelector('.content').textContent,
                !writtenHost.shadowRoot,
                writtenTemplate.getAttribute('shadowrootmode'),
                writtenTemplate.content.querySelector('.content').textContent,
                !fragmentHost.shadowRoot,
                fragmentTemplate.getAttribute('shadowrootmode'),
                fragmentTemplate.content.querySelector('.content').textContent
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "true|open|Content|true|open|Content|true|open|Content"
    );
}

#[test]
fn detached_contextual_fragment_uses_native_fragment_builder_after_method_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const body = d.body;
            const range = d.createRange();
            range.selectNode(body);
            Object.defineProperty(d.implementation, 'createHTMLDocument', {
                value() {
                    throw new Error('createHTMLDocument should not be called');
                },
                configurable: true
            });
            Object.defineProperty(d, 'createDocumentFragment', {
                value() {
                    throw new Error('createDocumentFragment should not be called');
                },
                configurable: true
            });
            Object.defineProperty(d, 'importNode', {
                value() {
                    throw new Error('importNode should not be called');
                },
                configurable: true
            });
            const fragment = range.createContextualFragment('<span id="native-fragment">ok</span>');
            Object.defineProperty(fragment, 'appendChild', {
                value() {
                    throw new Error('appendChild should not be called');
                },
                configurable: true
            });
            return [
                fragment.nodeType,
                fragment.firstChild.id,
                fragment.firstChild.ownerDocument === d,
                fragment.firstChild.parentNode === fragment,
                d.getElementById('native-fragment') === null
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "11|native-fragment|true|true|true");
}

#[test]
fn detached_document_title_setter_removes_native_children_after_child_nodes_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            d.title = 'old';
            const title = d.head.firstChild;
            Object.defineProperty(title, 'childNodes', {
                value: [],
                configurable: true
            });
            d.title = 'new';
            delete title.childNodes;
            return [
                d.title,
                title.textContent,
                title.firstChild.data,
                title.firstChild.nextSibling === null
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "new|new|new|true");
}

#[test]
fn detached_deep_clone_uses_native_children_after_child_nodes_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('section');
            const child = d.createElement('span');
            const text = d.createTextNode('native-clone');
            parent.id = 'clone-parent';
            child.id = 'clone-child';
            child.appendChild(text);
            parent.appendChild(child);
            d.body.appendChild(parent);
            Object.defineProperty(parent, 'childNodes', {
                value: [],
                configurable: true
            });
            const cloned = parent.cloneNode(true);
            return [
                cloned !== parent,
                cloned.childNodes.length,
                cloned.firstChild !== child,
                cloned.firstChild.id,
                cloned.firstChild.firstChild.data,
                cloned.ownerDocument === d
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|1|true|clone-child|native-clone|true");
}

#[test]
fn detached_html_document_deep_clone_copies_document_children() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            d.head.firstChild.textContent = 'Deep clone';
            const section = d.createElement('section');
            const text = d.createTextNode('body text');
            section.id = 'cloned-section';
            section.appendChild(text);
            d.body.appendChild(section);

            const cloned = d.cloneNode(true);
            return [
                cloned !== d,
                cloned.documentElement !== d.documentElement,
                cloned.childNodes.length,
                cloned.doctype && cloned.doctype.name,
                cloned.head.firstChild.textContent,
                cloned.body.firstChild.id,
                cloned.body.firstChild.firstChild.data,
                cloned.body.firstChild.ownerDocument === cloned
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "true|true|2|html|Deep clone|cloned-section|body text|true"
    );
}

#[test]
fn live_html_document_deep_clone_tag_name_collection_uses_cloned_tree() {
    let status = eval(
        r#"
        (() => {
            const html = document.createElement('html');
            const body = document.createElement('body');
            html.appendChild(body);
            document.appendChild(html);
            const marker = document.createElement('div');
            marker.id = 'live-document-clone-marker';
            body.appendChild(marker);

            const cloned = document.cloneNode(true);
            const bodies = cloned.getElementsByTagName('body');
            return [
                bodies.length,
                bodies[0] === cloned.body,
                cloned.getElementById('live-document-clone-marker') !== null
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "1|true|true");
}

#[test]
fn detached_clone_uses_native_node_type_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const element = d.createElement('span');
            const text = d.createTextNode('native-node-type');
            element.id = 'native-node-type-clone';
            element.appendChild(text);
            d.body.appendChild(element);
            Object.defineProperty(element, 'nodeType', {
                get() { throw new Error('element nodeType should not be read'); },
                configurable: true
            });
            Object.defineProperty(text, 'nodeType', {
                get() { throw new Error('text nodeType should not be read'); },
                configurable: true
            });
            const cloned = element.cloneNode(true);
            return [
                cloned.localName,
                cloned.id,
                cloned.firstChild.nodeType,
                cloned.firstChild.data,
                cloned.ownerDocument === d
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "span|native-node-type-clone|3|native-node-type|true"
    );
}

#[test]
fn detached_adopt_uses_native_node_type_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d1 = document.implementation.createHTMLDocument('');
            const d2 = document.implementation.createHTMLDocument('');
            const element = d1.createElement('span');
            element.id = 'native-node-type-adopt';
            d1.body.appendChild(element);
            Object.defineProperty(element, 'nodeType', {
                get() { throw new Error('adopt nodeType should not be read'); },
                configurable: true
            });
            const adopted = d2.adoptNode(element);
            return [
                adopted === element,
                adopted.ownerDocument === d2,
                adopted.parentNode === null,
                d1.body.contains(adopted)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|false");
}

#[test]
fn detached_mutation_values_use_native_node_type_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('section');
            const first = d.createElement('span');
            const second = d.createElement('b');
            first.id = 'native-node-type-append';
            second.id = 'native-node-type-replace';
            d.body.appendChild(parent);
            for (const node of [first, second]) {
                Object.defineProperty(node, 'nodeType', {
                    get() { throw new Error('mutation nodeType should not be read'); },
                    configurable: true
                });
            }
            parent.append(first);
            const afterAppend = parent.firstChild === first && first.parentNode === parent;
            parent.replaceChildren(second);
            return [
                afterAppend,
                parent.childNodes.length,
                parent.firstChild === second,
                second.parentNode === parent,
                first.parentNode === null,
                parent.textContent
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|1|true|true|true|");
}

#[test]
fn detached_node_name_bridge_uses_native_metadata_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const element = d.createElement('span');
            const text = d.createTextNode('text');
            const comment = d.createComment('comment');
            d.body.append(element, text, comment);
            for (const node of [element, text, comment]) {
                Object.defineProperty(node, 'nodeName', {
                    get() { throw new Error('nodeName should not be read'); },
                    configurable: true
                });
            }
            return [
                globalThis.__moliNativeBridge.__detachedNodeName(element),
                globalThis.__moliNativeBridge.__detachedNodeName(text),
                globalThis.__moliNativeBridge.__detachedNodeName(comment)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "SPAN|#text|#comment");
}

#[test]
fn detached_misc_node_metadata_bridge_uses_native_values_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createDocument('urn:test', 'root');
            const doctype = document.implementation.createDocumentType('html', 'pub', 'sys');
            const pi = d.createProcessingInstruction('xml-stylesheet', 'href="x.css"');
            for (const [node, names] of [
                [doctype, ['name', 'publicId', 'systemId']],
                [pi, ['target']]
            ]) {
                for (const name of names) {
                    Object.defineProperty(node, name, {
                        get() { throw new Error(`${name} should not be read`); },
                        configurable: true
                    });
                }
            }
            const bridge = globalThis.__moliNativeBridge;
            return [
                bridge.__detachedDoctypeName(doctype),
                bridge.__detachedDoctypePublicId(doctype),
                bridge.__detachedDoctypeSystemId(doctype),
                bridge.__detachedProcessingInstructionTarget(pi)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "html|pub|sys|xml-stylesheet");
}

#[test]
fn detached_doctype_and_pi_clone_use_native_metadata_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createDocument('urn:test', 'root');
            const doctype = document.implementation.createDocumentType('html', 'pub', 'sys');
            const pi = d.createProcessingInstruction('xml-stylesheet', 'href="x.css"');
            for (const [node, names] of [
                [doctype, ['name', 'publicId', 'systemId']],
                [pi, ['target']]
            ]) {
                for (const name of names) {
                    Object.defineProperty(node, name, {
                        get() { throw new Error(`${name} should not be read during clone`); },
                        configurable: true
                    });
                }
            }
            const clonedDoctype = doctype.cloneNode(false);
            const clonedPi = pi.cloneNode(false);
            return [
                clonedDoctype.name,
                clonedDoctype.publicId,
                clonedDoctype.systemId,
                clonedPi.target,
                clonedPi.data
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "html|pub|sys|xml-stylesheet|href=\"x.css\"");
}

#[test]
fn detached_element_clone_uses_native_metadata_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const element = d.createElement('span');
            element.id = 'native-metadata-clone';
            element.setAttribute('data-native', 'ok');
            d.body.appendChild(element);
            for (const [name, value] of [
                ['localName', 'section'],
                ['tagName', 'SECTION'],
                ['nodeName', 'SECTION'],
                ['namespaceURI', 'urn:wrong'],
                ['prefix', 'wrong']
            ]) {
                Object.defineProperty(element, name, {
                    get() { return value; },
                    configurable: true
                });
            }
            const cloned = element.cloneNode(true);
            return [
                cloned.localName,
                cloned.tagName,
                cloned.namespaceURI,
                cloned.prefix,
                cloned.getAttribute('data-native'),
                cloned.ownerDocument === d
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "span|SPAN|http://www.w3.org/1999/xhtml||ok|true");
}

#[test]
fn detached_template_clone_uses_native_metadata_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const template = d.createElement('template');
            const child = d.createElement('span');
            child.id = 'inside';
            child.textContent = 'native-template';
            template.content.appendChild(child);
            d.body.appendChild(template);
            for (const [name, value] of [
                ['localName', 'div'],
                ['tagName', 'DIV'],
                ['nodeName', 'DIV']
            ]) {
                Object.defineProperty(template, name, {
                    get() { return value; },
                    configurable: true
                });
            }
            const fake = d.createElement('em');
            fake.id = 'fake';
            fake.textContent = 'fake-template';
            Object.defineProperty(template, 'content', {
                value: { childNodes: { 0: fake, length: 1 } },
                configurable: true
            });
            const cloned = template.cloneNode(true);
            return [
                cloned.localName,
                cloned.content.childNodes.length,
                cloned.content.firstChild && cloned.content.firstChild.localName,
                cloned.content.firstChild && cloned.content.firstChild.id,
                cloned.content.firstChild && cloned.content.firstChild.textContent
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "template|1|span|inside|native-template");
}

#[test]
fn live_template_adoption_preserves_shadow_root_adopted_style_sheets_array() {
    let status = eval(
        r#"
        (() => {
            const sheet = new CSSStyleSheet();
            sheet.replaceSync("div { color: blue }");
            const body = document.createElement('body');
            document.appendChild(body);
            const host = document.createElement('div');
            body.appendChild(host);
            const root = host.attachShadow({ mode: 'open' });
            root.innerHTML = '<div></div>';
            root.adoptedStyleSheets = [sheet];
            const adopted = root.adoptedStyleSheets;
            const template = document.createElement('template');
            body.appendChild(template);
            template.content.appendChild(host);
            const afterTemplate = [
                host.ownerDocument !== document,
                root.firstChild.ownerDocument === host.ownerDocument,
                root.adoptedStyleSheets === adopted,
                root.adoptedStyleSheets.length,
                root.adoptedStyleSheets[0] === sheet
            ].join(',');
            body.appendChild(host);
            const afterDocument = [
                host.ownerDocument === document,
                root.firstChild.ownerDocument === document,
                root.adoptedStyleSheets === adopted,
                root.adoptedStyleSheets.length,
                root.adoptedStyleSheets[0] === sheet
            ].join(',');
            const iframe = document.createElement('iframe');
            body.appendChild(iframe);
            iframe.contentDocument.body.appendChild(host);
            const afterIframe = [
                host.ownerDocument === iframe.contentDocument,
                root.firstChild.ownerDocument === iframe.contentDocument,
                root.adoptedStyleSheets === adopted,
                root.adoptedStyleSheets.length
            ].join(',');
            return afterTemplate + '|' + afterDocument + '|' + afterIframe;
        })()
        "#,
    );
    assert_eq!(
        status,
        "true,true,true,1,true|true,true,true,1,true|true,true,true,0"
    );
}

#[test]
fn detached_owner_document_reads_follow_native_tree_after_native_adoption() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d1 = document.implementation.createHTMLDocument('');
                const d2 = document.implementation.createHTMLDocument('');
                d1.body.id = 'native-owner-body-one';
                d2.body.id = 'native-owner-body-two';
                const child = d1.createElement('article');
                child.id = 'native-owner-child';
                d1.body.appendChild(child);
                globalThis.__nativeOwnerDocOne = d1;
                globalThis.__nativeOwnerDocTwo = d2;
                globalThis.__nativeOwnerChild = child;
                return child.ownerDocument === d1 && child.parentNode === d1.body
                    ? 'ok'
                    : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let body_one = element_handle_by_id(&vm, "native-owner-body-one");
    let body_two = element_handle_by_id(&vm, "native-owner-body-two");
    let child = element_handle_by_id(&vm, "native-owner-child");
    {
        let dom_host = vm.document_runtime.dom_host_mut();
        assert!(dom_host.remove_child(body_one, child));
        assert!(dom_host.insert_before(body_two, child, None));
    }

    let status = vm
        .eval(
            r#"
            (() => {
                const d1 = globalThis.__nativeOwnerDocOne;
                const d2 = globalThis.__nativeOwnerDocTwo;
                const child = globalThis.__nativeOwnerChild;
                return [
                    child.ownerDocument === d2,
                    child.ownerDocument === d1,
                    child.parentNode === d2.body
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed ownerDocument reads should evaluate");
    assert_eq!(status, "true|false|true");
}

#[test]
fn detached_owner_document_adoption_ignores_stale_child_projection() {
    let (status, vm) = eval_with_vm(
        r#"
        (() => {
            const d1 = document.implementation.createHTMLDocument('');
            const d2 = document.implementation.createHTMLDocument('');
            d2.body.id = 'native-owner-target-body';
            const parent = d1.createElement('section');
            const child = d1.createElement('span');
            parent.id = 'native-owner-projection-parent';
            child.id = 'native-owner-projection-child';
            parent.appendChild(child);
            d1.body.appendChild(parent);
            const projected = parent.childNodes;
            projected[1] = { nodeType: 1, id: 'stale-owner-child' };
            projected.length = 2;
            const returned = d2.adoptNode(parent);
            return [
                returned === parent,
                parent.ownerDocument === d2,
                child.ownerDocument === d2,
                parent.childNodes.length,
                parent.firstChild === child
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|1|true");

    let body = element_handle_by_id(&vm, "native-owner-target-body");
    let parent = element_handle_by_id(&vm, "native-owner-projection-parent");
    let child = element_handle_by_id(&vm, "native-owner-projection-child");
    let dom = vm.document_runtime.dom_host().dom();
    let owner_document = dom.node(body).and_then(|node| node.owner_document());
    assert_eq!(
        dom.node(parent).and_then(|node| node.owner_document()),
        owner_document
    );
    assert_eq!(
        dom.node(child).and_then(|node| node.owner_document()),
        owner_document
    );
}

#[test]
fn detached_adopt_node_uses_native_parent_and_children_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d1 = document.implementation.createHTMLDocument('');
            const d2 = document.implementation.createHTMLDocument('');
            const parent = d1.createElement('section');
            const child = d1.createElement('span');
            parent.id = 'adopt-parent';
            child.id = 'adopt-child';
            parent.appendChild(child);
            d1.body.appendChild(parent);
            Object.defineProperty(parent, 'parentNode', {
                get() {
                    throw new Error('tampered parentNode');
                },
                configurable: true
            });
            Object.defineProperty(parent, 'childNodes', {
                value: [],
                configurable: true
            });
            const returned = d2.adoptNode(parent);
            delete parent.parentNode;
            delete parent.childNodes;
            return [
                returned === parent,
                parent.ownerDocument === d2,
                child.ownerDocument === d2,
                parent.parentNode === null,
                parent.firstChild === child,
                child.parentNode === parent,
                d1.body.contains(parent)
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|true|true|true|true|false");
}

#[test]
fn detached_get_root_node_follows_native_tree_after_native_adoption() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d1 = document.implementation.createHTMLDocument('');
                const d2 = document.implementation.createHTMLDocument('');
                d1.body.id = 'native-root-body-one';
                d2.body.id = 'native-root-body-two';
                const child = d1.createElement('article');
                child.id = 'native-root-child';
                d1.body.appendChild(child);
                globalThis.__nativeRootDocOne = d1;
                globalThis.__nativeRootDocTwo = d2;
                globalThis.__nativeRootChild = child;
                return child.getRootNode() === d1 ? 'ok' : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let body_one = element_handle_by_id(&vm, "native-root-body-one");
    let body_two = element_handle_by_id(&vm, "native-root-body-two");
    let child = element_handle_by_id(&vm, "native-root-child");
    {
        let dom_host = vm.document_runtime.dom_host_mut();
        assert!(dom_host.remove_child(body_one, child));
        assert!(dom_host.insert_before(body_two, child, None));
    }

    let status = vm
        .eval(
            r#"
            (() => {
                const d1 = globalThis.__nativeRootDocOne;
                const d2 = globalThis.__nativeRootDocTwo;
                const child = globalThis.__nativeRootChild;
                return [
                    child.getRootNode() === d2,
                    child.getRootNode() === d1,
                    child.parentNode === d2.body
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed getRootNode should evaluate");
    assert_eq!(status, "true|false|true");
}

#[test]
fn detached_queries_use_native_child_tree_after_native_move() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const oldParent = d.createElement('section');
                const newParent = d.createElement('section');
                const first = d.createElement('span');
                const target = d.createElement('span');
                oldParent.id = 'native-query-old-parent';
                newParent.id = 'native-query-new-parent';
                first.id = 'native-query-first';
                target.id = 'native-query-target';
                first.className = 'native-query';
                target.className = 'native-query';
                oldParent.appendChild(first);
                oldParent.appendChild(target);
                d.body.appendChild(oldParent);
                d.body.appendChild(newParent);
                globalThis.__nativeQueryDoc = d;
                globalThis.__nativeQueryOldParent = oldParent;
                globalThis.__nativeQueryNewParent = newParent;
                globalThis.__nativeQueryFirst = first;
                globalThis.__nativeQueryTarget = target;
                return oldParent.querySelectorAll('.native-query').length === 2 ? 'ok' : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let old_parent = element_handle_by_id(&vm, "native-query-old-parent");
    let new_parent = element_handle_by_id(&vm, "native-query-new-parent");
    let target = element_handle_by_id(&vm, "native-query-target");
    {
        let dom_host = vm.document_runtime.dom_host_mut();
        assert!(dom_host.remove_child(old_parent, target));
        assert!(dom_host.insert_before(new_parent, target, None));
    }

    let status = vm
        .eval(
            r#"
            (() => {
                const d = globalThis.__nativeQueryDoc;
                const oldParent = globalThis.__nativeQueryOldParent;
                const newParent = globalThis.__nativeQueryNewParent;
                const first = globalThis.__nativeQueryFirst;
                const target = globalThis.__nativeQueryTarget;
                const all = d.querySelectorAll('.native-query');
                return [
                    oldParent.querySelector('#native-query-target') === null,
                    oldParent.querySelectorAll('.native-query').length,
                    oldParent.getElementsByTagName('span').length,
                    newParent.querySelector('#native-query-target') === target,
                    newParent.getElementsByClassName('native-query').item(0) === target,
                    all.length,
                    all[0] === first,
                    all[1] === target,
                    d.getElementById('native-query-target') === target
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed detached queries should evaluate");
    assert_eq!(status, "true|1|1|true|true|2|true|true|true");
}

#[test]
fn detached_query_selector_attribute_operators_match_public_dom_surface() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const target = d.createElement('span');
            const miss = d.createElement('span');
            target.id = 'attribute-selector-target';
            target.setAttribute('data-eq', 'exact');
            target.setAttribute('data-word', 'alpha beta');
            target.setAttribute('data-prefix', 'alphabet');
            target.setAttribute('data-suffix', 'betalpha');
            target.setAttribute('data-substring', 'xxbetxx');
            target.setAttribute('lang', 'en-US');
            miss.setAttribute('data-word', 'alphabet');
            d.body.appendChild(miss);
            d.body.appendChild(target);

            return [
                d.querySelector('[data-eq="exact"]') === target,
                d.querySelector('[data-word~="beta"]') === target,
                d.querySelector('[data-prefix^="alpha"]') === target,
                d.querySelector('[data-suffix$="alpha"]') === target,
                d.querySelector('[data-substring*="bet"]') === target,
                d.querySelector('[lang|="en"]') === target,
                d.querySelector('[data-word~="alp"]') === null,
                d.querySelector('[data-prefix^=""]') === null,
                d.querySelector('[data-suffix$=""]') === null,
                d.querySelector('[data-substring*=""]') === null,
                d.querySelectorAll('[data-word~="beta"]').length
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "true|true|true|true|true|true|true|true|true|true|1"
    );
}

#[test]
fn detached_query_selector_all_returns_nodelist_and_element_matches_surface() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const section = d.createElement('section');
            const target = d.createElement('span');
            target.id = 'detached-match-target';
            target.className = 'hit';
            section.appendChild(target);
            d.body.appendChild(section);

            let invalid = null;
            try {
                target.matches(':not(');
            } catch (error) {
                invalid = error.name;
            }

            const documentList = d.querySelectorAll('.hit');
            const elementList = section.querySelectorAll('.hit');
            return [
                Object.prototype.toString.call(documentList),
                documentList.constructor?.name ?? null,
                documentList instanceof NodeList,
                typeof documentList.namedItem,
                Object.prototype.toString.call(elementList),
                elementList instanceof NodeList,
                elementList.item(0) === target,
                target.matches('body section > span.hit'),
                !target.webkitMatchesSelector('[class^=""]'),
                Object.prototype.hasOwnProperty.call(target, 'matches'),
                typeof target.matches,
                typeof target.webkitMatchesSelector,
                invalid
            ].join('|');
        })()
        "#,
    );
    assert_eq!(
        status,
        "[object NodeList]|NodeList|true|undefined|[object NodeList]|true|true|true|true|false|function|function|SyntaxError"
    );
}

#[test]
fn detached_collection_wrappers_do_not_expose_internal_data_property() {
    // This is a reflection-surface regression, not just a collection behavior
    // test. Detached collection declarations carry their backing data through
    // method callback data; that backing object must not appear as a `"data"`
    // own property on NodeList or HTMLCollection wrappers.
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const parent = d.createElement('section');
            const target = d.createElement('span');
            parent.appendChild(target);
            d.body.appendChild(parent);

            const childNodes = parent.childNodes;
            const queryList = parent.querySelectorAll('span');
            const children = parent.children;
            const tagCollection = parent.getElementsByTagName('span');

            const hasOwnData = value =>
                Object.prototype.hasOwnProperty.call(value, 'data') ||
                Object.getOwnPropertyNames(value).includes('data');

            return [
                hasOwnData(childNodes),
                hasOwnData(queryList),
                hasOwnData(children),
                hasOwnData(tagCollection),
                childNodes.item(0) === target,
                queryList.item(0) === target,
                children.item(0) === target,
                tagCollection.namedItem('missing') === null
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "false|false|false|false|true|true|true|true");
}

#[test]
fn detached_query_selector_target_decodes_document_fragment() {
    let mut vm = new_vm_with_url("https://detached-native.test/page.html#foo%20%E4%BD%A0");
    let status = vm
        .eval(
            r#"
        (() => {
            const d = new DOMParser().parseFromString(
                '<!doctype html><html><body><span id="foo 你"></span></body></html>',
                'text/html'
            );
            const target = d.getElementById('foo 你');
            return [
                d.URL,
                d.querySelector(':target') === target,
                target.matches(':target')
            ].join('|');
        })()
        "#,
        )
        .expect("detached :target fragment decoding should evaluate");
    assert_eq!(
        status,
        "https://detached-native.test/page.html#foo%20%E4%BD%A0|true|true"
    );
}

#[test]
fn detached_query_selector_target_uses_native_selector_engine_for_complex_selectors() {
    let mut vm = new_vm_with_url("https://detached-native.test/page.html#target");
    let status = vm
        .eval(
            r#"
        (() => {
            const d = new DOMParser().parseFromString(
                '<!doctype html><html><body><section id="target" class="hit" data-x="ok"><p></p></section></body></html>',
                'text/html'
            );
            const target = d.getElementById('target');
            const child = target.firstElementChild;
            return [
                d.querySelector('section.hit[data-x]:target') === target,
                d.querySelector(':target > p') === child,
                target.matches(':not(:target)'),
                target.matches('#target:target.hit')
            ].join('|');
        })()
        "#,
        )
        .expect("detached complex :target selector probe should evaluate");
    assert_eq!(status, "true|true|false|true");
}

#[test]
fn detached_get_element_by_id_uses_native_id_not_selector_fallback() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const target = d.createElement('div');
            target.setAttribute('id', 'a.b');
            d.body.appendChild(target);

            let idRead = false;
            Object.defineProperty(target, 'id', {
                configurable: true,
                get() {
                    idRead = true;
                    return 'wrong';
                }
            });

            return [
                d.getElementById('a.b') === target,
                d.querySelector('#a.b') === null,
                idRead
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|false");
}

#[test]
fn detached_collection_named_lookup_uses_native_attributes_after_property_tamper() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const hit = d.createElement('div');
            const miss = d.createElement('div');
            hit.setAttribute('id', 'real-id');
            d.body.appendChild(hit);
            d.body.appendChild(miss);

            let idRead = false;
            Object.defineProperty(hit, 'id', {
                configurable: true,
                get() {
                    idRead = true;
                    return 'wrong-id';
                }
            });
            Object.defineProperty(miss, 'id', {
                configurable: true,
                get() {
                    idRead = true;
                    return 'fake-id';
                }
            });

            const collection = d.getElementsByTagName('div');
            return [
                collection.namedItem('real-id') === hit,
                collection.namedItem('fake-id') === null,
                idRead
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "true|true|false");
}

#[test]
fn detached_form_control_name_reflects_native_attribute_for_collections() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const form = d.createElement('form');
            const input = d.createElement('input');
            form.appendChild(input);
            d.body.appendChild(form);

            input.name = 'native-name';
            let nameRead = false;
            Object.defineProperty(input, 'name', {
                configurable: true,
                get() {
                    nameRead = true;
                    return 'wrong-name';
                }
            });

            const controls = form.elements;
            return [
                input.getAttribute('name'),
                controls.namedItem('native-name') === input,
                controls.namedItem('wrong-name') === null,
                nameRead
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "native-name|true|true|false");
}

#[test]
fn detached_imported_form_control_name_reflects_native_attribute() {
    let status = eval(
        r#"
        (() => {
            const d = document.implementation.createHTMLDocument('');
            const form = d.createElement('form');
            const live = document.createElement('input');
            live.name = 'live-name';
            const imported = d.importNode(live, false);
            form.appendChild(imported);
            d.body.appendChild(form);

            imported.name = 'detached-name';
            let nameRead = false;
            Object.defineProperty(imported, 'name', {
                configurable: true,
                get() {
                    nameRead = true;
                    return 'wrong-name';
                }
            });

            return [
                imported.getAttribute('name'),
                form.elements.namedItem('detached-name') === imported,
                form.elements.namedItem('wrong-name') === null,
                nameRead
            ].join('|');
        })()
        "#,
    );
    assert_eq!(status, "detached-name|true|true|false");
}

#[test]
fn detached_get_elements_use_native_metadata_after_property_tamper() {
    let mut vm = new_vm();
    let status = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const target = d.createElement('span');
                const foreign = d.createElementNS('http://www.w3.org/2000/svg', 'svg');
                target.id = 'native-elements-target';
                target.className = 'native-elements hit';
                target.setAttribute('name', 'native-name');
                foreign.setAttribute('name', 'native-name');
                d.body.appendChild(target);
                d.body.appendChild(foreign);

                let getAttributeCalled = false;
                target.getAttribute = () => {
                    getAttributeCalled = true;
                    return null;
                };
                Object.defineProperty(target, 'localName', {
                    configurable: true,
                    get() { return 'section'; }
                });
                Object.defineProperty(target, 'namespaceURI', {
                    configurable: true,
                    get() { return null; }
                });
                Object.defineProperty(target, 'className', {
                    configurable: true,
                    get() { return 'tampered'; }
                });
                Object.defineProperty(target, 'nodeType', {
                    configurable: true,
                    get() { throw new Error('getElements nodeType should not be read'); }
                });

                return [
                    d.getElementsByTagName('span')[0] === target,
                    d.body.getElementsByTagName('span')[0] === target,
                    d.getElementsByTagNameNS('http://www.w3.org/1999/xhtml', 'span')[0] === target,
                    d.body.getElementsByTagNameNS('http://www.w3.org/1999/xhtml', 'span')[0] === target,
                    d.getElementsByClassName('native-elements hit')[0] === target,
                    d.body.getElementsByClassName('native-elements')[0] === target,
                    d.getElementsByName('native-name').length === 1 &&
                        d.getElementsByName('native-name')[0] === target,
                    getAttributeCalled
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed detached getElementsBy* should evaluate");
    assert_eq!(status, "true|true|true|true|true|true|true|false");
}

#[test]
fn detached_xpath_snapshot_uses_native_child_tree_after_native_move() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const oldParent = d.createElement('section');
                const newParent = d.createElement('section');
                const first = d.createElement('span');
                const target = d.createElement('span');
                oldParent.id = 'native-xpath-old-parent';
                newParent.id = 'native-xpath-new-parent';
                first.id = 'native-xpath-first';
                target.id = 'native-xpath-target';
                oldParent.appendChild(first);
                oldParent.appendChild(target);
                d.body.appendChild(oldParent);
                d.body.appendChild(newParent);
                globalThis.__nativeXPathDoc = d;
                globalThis.__nativeXPathOldParent = oldParent;
                globalThis.__nativeXPathNewParent = newParent;
                globalThis.__nativeXPathTarget = target;
                return d.evaluate('count(//span)', d, null, XPathResult.NUMBER_TYPE).numberValue === 2
                    ? 'ok'
                    : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let old_parent = element_handle_by_id(&vm, "native-xpath-old-parent");
    let new_parent = element_handle_by_id(&vm, "native-xpath-new-parent");
    let target = element_handle_by_id(&vm, "native-xpath-target");
    {
        let dom_host = vm.document_runtime.dom_host_mut();
        assert!(dom_host.remove_child(old_parent, target));
        assert!(dom_host.insert_before(new_parent, target, None));
    }

    let status = vm
        .eval(
            r#"
            (() => {
                const d = globalThis.__nativeXPathDoc;
                const oldParent = globalThis.__nativeXPathOldParent;
                const newParent = globalThis.__nativeXPathNewParent;
                const target = globalThis.__nativeXPathTarget;
                const all = d.evaluate('//span', d, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE);
                const oldCount = d.evaluate('count(.//span)', oldParent, null, XPathResult.NUMBER_TYPE);
                const newFirst = d.evaluate('.//span', newParent, null, XPathResult.FIRST_ORDERED_NODE_TYPE);
                return [
                    all.snapshotLength,
                    all.snapshotItem(0).id,
                    all.snapshotItem(1) === target,
                    oldCount.numberValue,
                    newFirst.singleNodeValue === target
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed detached XPath should evaluate");
    assert_eq!(status, "2|native-xpath-first|true|1|true");
}

#[test]
fn detached_xpath_snapshot_uses_native_metadata_after_property_tamper() {
    let mut vm = new_vm();
    let status = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const p = d.createElement('p');
                const text = d.createTextNode('native text');
                p.id = 'native-xpath-metadata';
                p.setAttribute('data-native', 'yes');
                p.appendChild(text);
                d.body.appendChild(p);

                let getAttributeNamesCalled = false;
                let getAttributeCalled = false;
                let getAttributeNodeCalled = false;
                p.getAttributeNames = () => {
                    getAttributeNamesCalled = true;
                    throw new Error('tampered getAttributeNames');
                };
                p.getAttribute = () => {
                    getAttributeCalled = true;
                    throw new Error('tampered getAttribute');
                };
                p.getAttributeNode = () => {
                    getAttributeNodeCalled = true;
                    throw new Error('tampered getAttributeNode');
                };
                Object.defineProperty(p, 'localName', {
                    configurable: true,
                    get() { return 'section'; }
                });
                Object.defineProperty(p, 'nodeType', {
                    configurable: true,
                    get() { throw new Error('xpath element nodeType should not be read'); }
                });
                Object.defineProperty(text, 'nodeValue', {
                    configurable: true,
                    get() { return 'tampered text'; }
                });
                Object.defineProperty(text, 'nodeType', {
                    configurable: true,
                    get() { throw new Error('xpath text nodeType should not be read'); }
                });

                const attr = d.evaluate(
                    '//p/@data-native',
                    d,
                    null,
                    XPathResult.FIRST_ORDERED_NODE_TYPE
                ).singleNodeValue;
                const textValue = d.evaluate('string(//p)', d, null, XPathResult.STRING_TYPE).stringValue;
                const pNode = d.evaluate('//p', d, null, XPathResult.FIRST_ORDERED_NODE_TYPE).singleNodeValue;
                const sectionCount = d.evaluate('count(//section)', d, null, XPathResult.NUMBER_TYPE).numberValue;
                return [
                    attr && attr.name,
                    attr && attr.value,
                    attr && attr.ownerElement === p,
                    textValue,
                    pNode === p,
                    sectionCount,
                    getAttributeNamesCalled,
                    getAttributeCalled,
                    getAttributeNodeCalled
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed detached XPath metadata should evaluate");
    assert_eq!(
        status,
        "data-native|yes|true|native text|true|0|false|false|false"
    );
}

#[test]
fn detached_normalize_uses_native_child_order() {
    let mut vm = new_vm();
    let setup = vm
        .eval(
            r#"
            (() => {
                const d = document.implementation.createHTMLDocument('');
                const parent = d.createElement('div');
                const first = d.createTextNode('native-normalize-a');
                const span = d.createElement('span');
                const second = d.createTextNode('native-normalize-b');
                parent.id = 'native-normalize-parent';
                span.id = 'native-normalize-span';
                parent.appendChild(first);
                parent.appendChild(span);
                parent.appendChild(second);
                d.body.appendChild(parent);
                globalThis.__nativeNormalizeParent = parent;
                globalThis.__nativeNormalizeFirst = first;
                globalThis.__nativeNormalizeSecond = second;
                globalThis.__nativeNormalizeSpan = span;
                return parent.childNodes.length === 3 ? 'ok' : 'bad';
            })()
            "#,
        )
        .expect("setup should evaluate");
    assert_eq!(setup, "ok");

    let parent = element_handle_by_id(&vm, "native-normalize-parent");
    let span = element_handle_by_id(&vm, "native-normalize-span");
    let second = text_handle_by_value(&vm, "native-normalize-b");
    {
        let dom_host = vm.document_runtime.dom_host_mut();
        assert!(dom_host.remove_child(parent, second));
        assert!(dom_host.insert_before(parent, second, Some(span)));
    }

    let status = vm
        .eval(
            r#"
            (() => {
                const parent = globalThis.__nativeNormalizeParent;
                const first = globalThis.__nativeNormalizeFirst;
                const second = globalThis.__nativeNormalizeSecond;
                const span = globalThis.__nativeNormalizeSpan;
                parent.normalize();
                const nodes = parent.childNodes;
                return [
                    nodes.length,
                    nodes[0] === first,
                    first.data,
                    nodes[1] === span,
                    second.parentNode === null,
                    span.previousSibling === first
                ].join('|');
            })()
            "#,
        )
        .expect("native-backed normalize should evaluate");
    assert_eq!(
        status,
        "2|true|native-normalize-anative-normalize-b|true|true|true"
    );
}
