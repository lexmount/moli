use super::auxiliary_page_identity::open_auxiliary;
use super::*;

// A ScriptVm popup can share its native host with its opener. Publish and
// attach the auxiliary CDP target so these cases exercise two actual Pages.
#[tokio::test(flavor = "multi_thread")]
async fn borrowed_event_target_methods_use_the_native_node_owner() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let (producer_id, _producer) = open_auxiliary(
        addr,
        &mut opener,
        "window.foreignNode = p.document.createElement('div'); window.listenerCalls = 0",
    )
    .await;

    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            2,
            r#"(() => {
                const listener = () => { listenerCalls++; };
                EventTarget.prototype.addEventListener.call(foreignNode, 'test', listener);
                const registered = EventTarget.prototype.dispatchEvent.call(foreignNode, new Event('test')) && listenerCalls === 1;
                EventTarget.prototype.removeEventListener.call(foreignNode, 'test', listener);
                foreignNode.dispatchEvent(new Event('test'));
                const removed = listenerCalls === 1;

                const nativeProxy = p.document.implementation.createHTMLDocument('').createElement('select');
                let proxyCalls = 0;
                EventTarget.prototype.addEventListener.call(nativeProxy, 'test', () => { proxyCalls++; });
                const registeredNativeProxy = EventTarget.prototype.dispatchEvent.call(nativeProxy, new Event('test')) && proxyCalls === 1;

                const localNode = document.createElement('div');
                let reverseCalls = 0;
                const reverseListener = () => { reverseCalls++; };
                p.EventTarget.prototype.addEventListener.call(localNode, 'test', reverseListener);
                p.EventTarget.prototype.dispatchEvent.call(localNode, new Event('test'));
                p.EventTarget.prototype.removeEventListener.call(localNode, 'test', reverseListener);
                localNode.dispatchEvent(new Event('test'));

                let conversions = 0, proxyError, forgedError;
                const type = {toString() { conversions++; return 'test'; }};
                try { EventTarget.prototype.addEventListener.call(new Proxy(foreignNode, {}), type, listener); }
                catch (error) { proxyError = error; }
                try { p.EventTarget.prototype.addEventListener.call(Object.create(foreignNode), type, listener); }
                catch (error) { forgedError = error; }
                return {
                    registered, removed, registeredNativeProxy,
                    reverseOwner: reverseCalls === 1,
                    proxyCalleeRealm: proxyError instanceof TypeError && !(proxyError instanceof p.TypeError),
                    forgedCalleeRealm: forgedError instanceof p.TypeError && !(forgedError instanceof TypeError),
                    receiverBeforeConversion: conversions === 0
                };
            })()"#,
        )
        .await,
        json!({
            "registered": true,
            "removed": true,
            "registeredNativeProxy": true,
            "reverseOwner": true,
            "proxyCalleeRealm": true,
            "forgedCalleeRealm": true,
            "receiverBeforeConversion": true
        })
    );
    evaluate_window_name_probe(&mut opener, 3, "p.close(); true").await;
    wait_for_target_list(addr, "the producer Page has closed", |targets| {
        !targets.iter().any(|target| target["id"] == producer_id)
    })
    .await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            4,
            "EventTarget.prototype.dispatchEvent.call(foreignNode, new Event('test')) && listenerCalls === 1"
        )
        .await,
        true
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_adoption_preserves_views_and_callbacks_across_pages() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let (producer_id, _producer) = open_auxiliary(
        addr,
        &mut opener,
        r#"
            window.sourceDocument = p.document;
            window.root = sourceDocument.createElement('section');
            window.unmoved = sourceDocument.createElement('div');
            sourceDocument.body.appendChild(unmoved);
            for (let index = 0; index < 1200; index++) {
                const child = sourceDocument.createElement('span');
                child.id = 'child-' + index;
                root.appendChild(child);
            }
            sourceDocument.body.appendChild(root);
            window.first = root.firstChild;
            window.last = root.lastChild;
            window.originalPrototype = Object.getPrototypeOf(root);
            window.children = root.childNodes;
            window.tags = root.getElementsByTagName('span');
            window.snapshot = root.querySelectorAll('span');
            window.mixed = sourceDocument.querySelectorAll('div, span');
            window.style = root.style;
            window.classes = root.classList;
            window.dataset = root.dataset;
            window.controller = new p.AbortController();
            window.listenerCalls = 0;
            root.addEventListener('owned', () => { listenerCalls++; }, {signal: controller.signal});
            window.realmListener = new p.Function('event', 'this.callbackDocument = document; this.eventInSourceRealm = event instanceof Event;');
            root.addEventListener('realm', realmListener)
        "#,
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            2,
            r#"(() => {
                const returnedOriginal = document.adoptNode(root) === root;
                document.body.appendChild(root);
                style.setProperty('color', 'red');
                classes.add('adopted');
                dataset.owner = 'target';
                root.dispatchEvent(new Event('owned'));
                root.dispatchEvent(new Event('realm'));
                const result = {
                    returnedOriginal,
                    prototypeRealm: Object.getPrototypeOf(root) === originalPrototype,
                    nativeOwners: root.ownerDocument === document && first.ownerDocument === document && last.ownerDocument === document,
                    liveSameObject: root.childNodes === children && children.length === 1200 && children[0] === first,
                    tags: tags.length === 1200 && tags[1199] === last,
                    staticItems: snapshot.length === 1200 && snapshot[0] === first && snapshot.item(1199) === last,
                    staticDescriptor: Object.getOwnPropertyDescriptor(snapshot, '0').value === first,
                    staticIterator: Array.from(snapshot)[1199] === last,
                    mixedOwners: mixed.length === 1201 && mixed[0] === unmoved && unmoved.ownerDocument === sourceDocument && mixed[1200] === last,
                    retainedViews: root.style === style && root.classList === classes && root.dataset === dataset && root.getAttribute('data-owner') === 'target' && root.className === 'adopted' && root.style.color === 'red',
                    listenerMoved: listenerCalls === 1,
                    callbackRealm: root.callbackDocument === sourceDocument && root.eventInSourceRealm === false
                };
                controller.abort();
                root.dispatchEvent(new Event('owned'));
                result.signalRemovesMovedListener = listenerCalls === 1;
                first.remove();
                result.liveMutation = children.length === 1199 && tags.length === 1199 && children[0].id === 'child-1';
                result.staticRemovedNode = snapshot[0] === first && first.parentNode === null && first.ownerDocument === document;
                sourceDocument.adoptNode(root);
                sourceDocument.body.appendChild(root);
                result.secondAdoption = root.ownerDocument === sourceDocument && last.ownerDocument === sourceDocument && snapshot[1199] === last && tags[1198] === last && first.ownerDocument === document;
                document.adoptNode(root);
                document.body.appendChild(root);
                result.thirdAdoption = root.ownerDocument === document && last.ownerDocument === document && snapshot[1199] === last && mixed[0] === unmoved && unmoved.ownerDocument === sourceDocument;
                return result;
            })()"#,
        )
        .await,
        json!({
            "returnedOriginal": true,
            "prototypeRealm": true,
            "nativeOwners": true,
            "liveSameObject": true,
            "tags": true,
            "staticItems": true,
            "staticDescriptor": true,
            "staticIterator": true,
            "mixedOwners": true,
            "retainedViews": true,
            "listenerMoved": true,
            "callbackRealm": true,
            "signalRemovesMovedListener": true,
            "liveMutation": true,
            "staticRemovedNode": true,
            "secondAdoption": true,
            "thirdAdoption": true
        })
    );
    evaluate_window_name_probe(&mut opener, 3, "p.close(); true").await;
    wait_for_target_list(addr, "adoption's producer Page has closed", |targets| {
        !targets.iter().any(|target| target["id"] == producer_id)
    })
    .await;
    send_cdp_command(
        &mut opener,
        4,
        "HeapProfiler.collectGarbage",
        None,
        json!({}),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            5,
            r#"(() => {
                root.dispatchEvent(new Event('realm'));
                const callbackRealm = root.callbackDocument === sourceDocument && root.eventInSourceRealm === false;
                root.removeEventListener('realm', realmListener);
                root.callbackDocument = null;
                root.dispatchEvent(new Event('realm'));
                root.appendChild(document.createElement('span'));
                return {
                    movedOwner: root.ownerDocument === document && last.ownerDocument === document,
                    mixedOwners: mixed[0] === unmoved && unmoved.ownerDocument === sourceDocument && mixed[1200] === last,
                    removedSnapshot: snapshot[0] === first && first.parentNode === null && first.ownerDocument === document,
                    liveMutation: children.length === 1200 && tags.length === 1200,
                    callbackRealm,
                    callbackRemoved: root.callbackDocument === null
                };
            })()"#,
        )
        .await,
        json!({
            "movedOwner": true,
            "mixedOwners": true,
            "removedSnapshot": true,
            "liveMutation": true,
            "callbackRealm": true,
            "callbackRemoved": true
        })
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_closed_page_materializes_nodes_in_their_owner_realm() {
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let (producer_id, _producer) = open_auxiliary(
        addr,
        &mut opener,
        r#"
            window.retainedDocument = p.document;
            window.retainedBody = retainedDocument.body;
            window.producerElementPrototype = p.HTMLElement.prototype;
            retainedBody.innerHTML = '<article id="late" style="color:red"><b>original</b></article>'
        "#,
    )
    .await;
    evaluate_window_name_probe(&mut opener, 2, "p.close(); true").await;
    wait_for_target_list(addr, "the retained Document's Page has closed", |targets| {
        !targets.iter().any(|target| target["id"] == producer_id)
    })
    .await;
    send_cdp_command(
        &mut opener,
        3,
        "HeapProfiler.collectGarbage",
        None,
        json!({}),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            4,
            r#"(() => {
                const getter = Object.getOwnPropertyDescriptor(Node.prototype, 'firstChild').get;
                const child = getter.call(retainedBody);
                const freshStyle = getComputedStyle(child);
                child.dataset.afterClose = 'yes';
                child.firstChild.textContent = 'changed';
                return {
                    nativeOwner: child.ownerDocument === retainedDocument,
                    producerPrototype: producerElementPrototype.isPrototypeOf(child),
                    nativeRead: child.id === 'late' && child.tagName === 'ARTICLE',
                    nativeMutation: retainedBody.firstChild === child && retainedBody.textContent === 'changed' && child.getAttribute('data-after-close') === 'yes',
                    freshStyle: freshStyle.color === 'rgb(255, 0, 0)'
                };
            })()"#,
        )
        .await,
        json!({
            "nativeOwner": true,
            "producerPrototype": true,
            "nativeRead": true,
            "nativeMutation": true,
            "freshStyle": true
        })
    );
    abort_test_cdp_server(server).await;
}
