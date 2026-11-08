use super::*;

fn native_alias_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    template: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let handler = crate::util::new_null_prototype_object(scope);
    for name in ["get", "getPrototypeOf"] {
        let key = crate::util::v8str(scope, name);
        let value = template.get(scope, key.into()).unwrap();
        assert_eq!(
            handler.create_data_property(scope, key.into(), value),
            Some(true)
        );
    }
    handler
}

fn native_node_alias_test_vm(setup: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm("https://node-native-aliases.test/", "<!doctype html>");
    vm.eval(
        r#"
        globalThis.aliasTraps = 0;
        globalThis.aliasHandlerTemplate = {
          get() { aliasTraps++; throw Error('get trap'); },
          getPrototypeOf() { aliasTraps++; throw Error('prototype trap'); }
        };
        globalThis.assertAlias = (condition, message) => {
          if (!condition) throw Error(message);
        };
    "#,
    )
    .unwrap();
    vm.eval(&format!("(() => {{\n{setup}\n}})()")).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nodeAliasTargets");
        let targets = global.get(scope, key.into()).unwrap();
        let targets = v8::Local::<v8::Array>::try_from(targets).unwrap();
        let key = crate::util::v8str(scope, "aliasHandlerTemplate");
        let template = global.get(scope, key.into()).unwrap();
        let template = v8::Local::<v8::Object>::try_from(template).unwrap();
        let aliases = v8::Array::new(scope, targets.length() as i32);
        for index in 0..targets.length() {
            let target = targets.get_index(scope, index).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let backing = moli_webapi_declare::web_api_object_target(scope, target).unwrap();
            let handler = native_alias_handler(scope, template);
            let proxy = v8::Proxy::new(scope, backing, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(aliases.set_index(scope, index, proxy.into()), Some(true));
            if index == 0 {
                let handler = native_alias_handler(scope, template);
                let revoked = v8::Proxy::new(scope, backing, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, revoked).unwrap();
                revoked.revoke();
                let key = crate::util::v8str(scope, "revokedNodeAlias");
                assert_eq!(
                    global.create_data_property(scope, key.into(), revoked.into()),
                    Some(true)
                );
            }
        }
        let key = crate::util::v8str(scope, "nodeAliases");
        assert_eq!(
            global.create_data_property(scope, key.into(), aliases.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm
}

fn assert_node_private_state_is_backing_only(vm: &mut StandaloneScriptVmHarness) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        for name in ["nodeAliasTargets", "nodeAliases"] {
            let objects = global
                .get(scope, crate::util::v8str(scope, name).into())
                .unwrap();
            let objects = v8::Local::<v8::Array>::try_from(objects).unwrap();
            for index in 0..objects.length() {
                let object = objects.get_index(scope, index).unwrap();
                let object = v8::Local::<v8::Object>::try_from(object).unwrap();
                let backing = moli_webapi_declare::web_api_object_target(scope, object).unwrap();
                assert!(!backing.is_proxy());
                if object.is_proxy() {
                    for slot in [
                        crate::native_bridge::document::DETACHED_STATE_SLOT,
                        crate::native_bridge::document::DETACHED_NATIVE_HANDLE_SLOT,
                        crate::native_bridge::document::DETACHED_LIVE_DELEGATE_SLOT,
                        "__moliDetachedElementBridgePrototype",
                        "__moliNamedNodeMapCache",
                        "__moliAttrObjectCache",
                        "__moliAriaElementReference:aria-controls",
                        "__moliDetachedDOMImplementation",
                        "__moliDocumentAssociatedWindow",
                    ] {
                        assert!(
                            crate::util::get_private_value(scope, object, slot).is_none(),
                            "{name}[{index}] must not own {slot}"
                        );
                    }
                }
            }
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn native_node_factories_keep_private_state_only_on_backing_objects() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const detached = document.implementation.createHTMLDocument('');
        const parsed = new DOMParser().parseFromString(
          '<!doctype html><body><x-native></x-native><select><option>one</option></select>', 'text/html');
        globalThis.nodeAliasTargets = [
          detached.createElement('select'), parsed.body.firstChild, parsed.body.lastChild,
          detached.createTextNode('text'), detached.createDocumentFragment()
        ];
    "#,
    );
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let values = global
            .get(scope, crate::util::v8str(scope, "nodeAliasTargets").into())
            .unwrap();
        let values = v8::Local::<v8::Array>::try_from(values).unwrap();
        for index in 0..3 {
            let object =
                v8::Local::<v8::Object>::try_from(values.get_index(scope, index).unwrap()).unwrap();
            assert!(
                object.is_proxy(),
                "factory {index} must exercise a canonical native Proxy"
            );
            let backing = moli_webapi_declare::web_api_object_target(scope, object).unwrap();
            assert!(
                crate::util::get_private_object(
                    scope,
                    backing,
                    crate::native_bridge::document::DETACHED_STATE_SLOT
                )
                .is_some()
            );
            assert!(
                crate::util::get_private_value(
                    scope,
                    backing,
                    crate::native_bridge::document::DETACHED_NATIVE_HANDLE_SLOT
                )
                .is_some()
            );
            let canonical = crate::util::get_private_object(
                scope,
                backing,
                crate::dom_parser::DOM_PARSER_FOREIGN_NODE_SLOT,
            )
            .unwrap();
            assert_eq!(
                canonical, object,
                "backing retains canonical JS identity for factory {index}"
            );
        }
        Ok(())
    })
    .unwrap();
    assert_node_private_state_is_backing_only(&mut vm);
    vm.eval(r#"
        const n = Node.prototype;
        const text = Object.getOwnPropertyDescriptor(n, 'textContent');
        for (let i = 0; i < nodeAliases.length; i++) {
          const original = nodeAliasTargets[i], alias = nodeAliases[i];
          assertAlias(n.getRootNode.call(alias) === n.getRootNode.call(original), 'factory root ' + i);
          assertAlias(n.isEqualNode.call(n.cloneNode.call(alias, true), n.cloneNode.call(original, true)), 'factory clone ' + i);
          text.set.call(alias, 'updated');
          assertAlias(original.textContent === 'updated', 'factory state ' + i);
        }
        assertAlias(aliasTraps === 0, 'factory state lookup ran traps');
    "#).unwrap();
    assert_node_private_state_is_backing_only(&mut vm);
}

#[test]
fn detached_node_alias_arguments_preserve_native_identity() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const detached = document.implementation.createHTMLDocument('');
        globalThis.nodeAliasTargets = [
          detached.createElement('span'), detached.createElement('i'), detached.body
        ];
        nodeAliasTargets[0].textContent = 'child';
    "#,
    );
    vm.eval(
        r#"
        const [child, sibling, parent] = nodeAliasTargets;
        const [alias, siblingAlias, parentAlias] = nodeAliases;
        const n = Node.prototype;
        assertAlias(n.isSameNode.call(child, alias), 'same detached handle');
        assertAlias(n.isEqualNode.call(child, alias), 'equal detached node');
        assertAlias(n.appendChild.call(parentAlias, alias) === alias, 'append return identity');
        assertAlias(parent.firstChild === child, 'append reuses canonical node');
        assertAlias(n.contains.call(parentAlias, alias), 'contains alias');
        assertAlias(n.contains.call(parentAlias, child), 'contains canonical node');
        assertAlias(n.insertBefore.call(parentAlias, siblingAlias, alias) === siblingAlias,
                    'insert return identity');
        assertAlias(parent.firstChild === sibling && parent.lastChild === child &&
                    parent.childNodes.length === 2, 'insert reuses existing handles');
        assertAlias(n.removeChild.call(parentAlias, alias) === alias, 'remove return identity');
        assertAlias(child.parentNode === null && parent.childNodes.length === 1, 'remove alias');
        assertAlias(n.appendChild.call(document.body, alias) === alias, 'live append return identity');
        assertAlias(document.body.firstChild === child && child.ownerDocument === document,
                    'live append must adopt the existing handle');
        assertAlias(n.insertBefore.call(document.body, siblingAlias, alias) === siblingAlias,
                    'live insert return identity');
        assertAlias(document.body.firstChild === sibling && document.body.lastChild === child &&
                    document.body.childNodes.length === 2 && parent.childNodes.length === 0,
                    'live insert must move existing nodes');
        assertAlias(aliasTraps === 0, 'argument resolution ran a trap');
    "#,
    )
    .unwrap();
}

#[test]
fn detached_node_alias_setters_update_backing() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const detached = document.implementation.createHTMLDocument('');
        const element = detached.createElement('div');
        element.appendChild(detached.createElement('span'));
        globalThis.nodeAliasTargets = [detached.createTextNode('old'), element];
    "#,
    );
    vm.eval(
        r#"
        const [text, element] = nodeAliasTargets;
        const nodeValue = Object.getOwnPropertyDescriptor(Node.prototype, 'nodeValue');
        const textContent = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent');
        nodeValue.set.call(nodeAliases[0], 'node value');
        assertAlias(text.data === 'node value', 'nodeValue did not update backing');
        assertAlias(nodeValue.get.call(nodeAliases[0]) === 'node value', 'nodeValue readback');
        textContent.set.call(nodeAliases[0], 'text content');
        assertAlias(text.data === 'text content', 'textContent did not update CharacterData');
        textContent.set.call(nodeAliases[1], 'replacement');
        assertAlias(element.textContent === 'replacement' && element.childNodes.length === 1 &&
                    element.firstChild.nodeType === 3, 'textContent did not replace children');
        textContent.set.call(nodeAliases[1], null);
        assertAlias(element.childNodes.length === 0, 'textContent did not clear children');
        assertAlias(aliasTraps === 0, 'setter ran a trap');
    "#,
    )
    .unwrap();
}

#[test]
fn detached_node_alias_clone_and_root_preserve_detached_semantics() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const html = document.implementation.createHTMLDocument('alias clone');
        const xml = new DOMParser().parseFromString('<root><child>xml</child></root>', 'text/xml');
        const element = html.createElement('span');
        element.textContent = 'html';
        html.body.appendChild(element);
        const fragment = html.createDocumentFragment();
        fragment.appendChild(html.createTextNode('fragment'));
        globalThis.nodeAliasTargets = [html, xml, element, html.createTextNode('orphan'), fragment];
    "#,
    );
    vm.eval(
        r#"
        const n = Node.prototype;
        for (let i = 0; i < nodeAliases.length; i++) {
          const original = nodeAliasTargets[i];
          const alias = nodeAliases[i];
          assertAlias(n.getRootNode.call(alias) === n.getRootNode.call(original), 'root identity ' + i);
          assertAlias(n.getRootNode.call(alias, {composed: true}) === n.getRootNode.call(original),
                      'composed root identity ' + i);
          for (const deep of [false, true]) {
            const canonicalClone = n.cloneNode.call(original, deep);
            const aliasClone = n.cloneNode.call(alias, deep);
            assertAlias(aliasClone !== original && aliasClone !== alias, 'fresh clone ' + i);
            assertAlias(n.isEqualNode.call(canonicalClone, aliasClone), 'clone contents ' + i);
            assertAlias(Object.getPrototypeOf(canonicalClone) === Object.getPrototypeOf(aliasClone),
                        'clone prototype ' + i);
            if (original.nodeType === 9) {
              assertAlias(aliasClone.contentType === original.contentType, 'clone content type ' + i);
              assertAlias(aliasClone.characterSet === original.characterSet, 'clone character set ' + i);
              assertAlias(aliasClone.ownerDocument === null, 'document clone owner ' + i);
            } else {
              assertAlias(aliasClone.ownerDocument === original.ownerDocument, 'clone owner ' + i);
            }
          }
        }
        assertAlias(aliasTraps === 0, 'clone or getRootNode ran a trap');
    "#,
    )
    .unwrap();
}

#[test]
fn detached_node_aliases_follow_adoption_through_dom_methods() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const detached = document.implementation.createHTMLDocument('');
        globalThis.nodeAliasTargets = [detached.createElement('select'), detached.createElement('div')];
    "#,
    );
    vm.eval(
        r#"
        const n = Node.prototype;
        const textContent = Object.getOwnPropertyDescriptor(n, 'textContent');
        for (let i = 0; i < nodeAliases.length; i++) {
          const original = nodeAliasTargets[i];
          const alias = nodeAliases[i];
          assertAlias(n.isSameNode.call(alias, original), 'identity before adoption');
          const argument = i === 0 ? original : alias;
          assertAlias(document.adoptNode(argument) === argument, 'adopt return identity');
          assertAlias(n.appendChild.call(document.body, alias) === alias, 'append adopted alias');
          assertAlias(document.body.lastChild === original, 'adoption keeps canonical wrapper');
          assertAlias(n.isSameNode.call(alias, original), 'identity after adoption');
          assertAlias(n.contains.call(document.body, alias), 'live contains adopted alias');
          assertAlias(n.getRootNode.call(alias) === document, 'live root after adoption');
          textContent.set.call(alias, 'adopted');
          assertAlias(original.textContent === 'adopted' && original.ownerDocument === document &&
                      original.isConnected, 'live mutation through old alias');
        }
        assertAlias(document.body.childNodes.length === 2, 'adoption duplicated a node');
        assertAlias(aliasTraps === 0, 'adoption ran a trap');
    "#,
    )
    .unwrap();
    assert_node_private_state_is_backing_only(&mut vm);
}

#[test]
fn native_node_alias_caches_preserve_canonical_attribute_identity() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const detached = document.implementation.createHTMLDocument('');
        const select = detached.createElement('select');
        select.setAttribute('data-owner', 'before');
        globalThis.nodeAliasTargets = [select, detached];
    "#,
    );
    vm.eval(r#"
        const [element, doc] = nodeAliasTargets;
        const [alias, documentAlias] = nodeAliases;
        const attributes = Object.getOwnPropertyDescriptor(Element.prototype, 'attributes');
        assertAlias(attributes.get.call(alias) === element.attributes, 'shared NamedNodeMap');
        const attr = Element.prototype.getAttributeNode.call(alias, 'data-owner');
        assertAlias(attr === element.getAttributeNode('data-owner'), 'shared Attr cache');
        assertAlias(attr.ownerElement === element, 'canonical Attr owner');
        Element.prototype.setAttribute.call(alias, 'data-owner', 'after');
        assertAlias(attr.value === 'after', 'cached Attr follows native mutation');
        const added = doc.createAttribute('data-added');
        Element.prototype.setAttributeNode.call(alias, added);
        assertAlias(added.ownerElement === element, 'setAttributeNode canonical owner');
        assertAlias(Element.prototype.setAttributeNode.call(element, added) === added, 'same Attr through canonical receiver');
        assertAlias(Element.prototype.removeAttributeNode.call(alias, added) === added && added.ownerElement === null,
                    'remove Attr through alias');
        const controls = Object.getOwnPropertyDescriptor(Element.prototype, 'ariaControlsElements');
        const references = [element];
        controls.set.call(alias, references);
        assertAlias(controls.get.call(element) === controls.get.call(alias) && controls.get.call(element)[0] === element,
                    'shared ARIA references');
        controls.set.call(element, null);
        assertAlias(controls.get.call(alias) === null, 'ARIA references clear through canonical node');
        const implementation = Object.getOwnPropertyDescriptor(Document.prototype, 'implementation');
        assertAlias(implementation.get.call(documentAlias) === doc.implementation, 'shared DOMImplementation');
        assertAlias(aliasTraps === 0, 'cache lookup ran traps');
    "#).unwrap();
    assert_node_private_state_is_backing_only(&mut vm);
}

#[test]
fn native_node_alias_observers_report_canonical_targets() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const detached = document.implementation.createHTMLDocument('');
        globalThis.nodeAliasTargets = [detached.createElement('select')];
    "#,
    );
    vm.eval(
        r#"
        const element = nodeAliasTargets[0], alias = nodeAliases[0];
        const observer = new MutationObserver(() => {});
        observer.observe(alias, {attributes: true, childList: true});
        Element.prototype.setAttribute.call(alias, 'data-observed', 'yes');
        const option = element.ownerDocument.createElement('option');
        Node.prototype.appendChild.call(alias, option);
        const records = observer.takeRecords();
        assertAlias(records.length === 2 && records.every(record => record.target === element),
                    'observer reports canonical target');
        assertAlias(records[1].addedNodes[0] === option, 'observer reports canonical added node');
        observer.disconnect();
        assertAlias(aliasTraps === 0, 'observe ran traps');
    "#,
    )
    .unwrap();
    assert_node_private_state_is_backing_only(&mut vm);
}

#[test]
fn native_node_alias_results_use_the_canonical_receivers_realm() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const frame = document.createElement('iframe');
        document.body.appendChild(frame);
        const foreign = frame.contentDocument;
        const element = foreign.createElement('div');
        element.appendChild(foreign.createElement('span'));
        foreign.body.appendChild(element);
        globalThis.nodeAliasTargets = [element, foreign.body, foreign];
    "#,
    );
    vm.eval(r#"
        const n = Node.prototype;
        for (let i = 0; i < nodeAliases.length; i++) {
          const canonical = nodeAliasTargets[i], alias = nodeAliases[i];
          for (const name of ['ownerDocument', 'parentNode', 'firstChild', 'lastChild']) {
            const getter = Object.getOwnPropertyDescriptor(n, name).get;
            assertAlias(getter.call(alias) === getter.call(canonical), 'foreign ' + name + ' ' + i);
          }
          assertAlias(n.getRootNode.call(alias) === n.getRootNode.call(canonical), 'foreign root ' + i);
          const clone = n.cloneNode.call(alias, true), canonicalClone = n.cloneNode.call(canonical, true);
          assertAlias(Object.getPrototypeOf(clone) === Object.getPrototypeOf(canonicalClone), 'foreign clone realm ' + i);
        }
        const doc = nodeAliasTargets[2], documentAlias = nodeAliases[2];
        const defaultView = Object.getOwnPropertyDescriptor(Document.prototype, 'defaultView');
        assertAlias(defaultView.get.call(documentAlias) === doc.defaultView, 'foreign defaultView identity');
        const forms = Object.getOwnPropertyDescriptor(Document.prototype, 'forms');
        assertAlias(Object.getPrototypeOf(forms.get.call(documentAlias)) === Object.getPrototypeOf(doc.forms),
                    'foreign collection realm');
        assertAlias(aliasTraps === 0, 'foreign receiver lookup ran traps');
    "#).unwrap();
    assert_node_private_state_is_backing_only(&mut vm);
}

#[test]
fn node_alias_dom_calls_reject_author_and_revoked_proxies_without_traps() {
    let mut vm = native_node_alias_test_vm(
        r#"
        const detached = document.implementation.createHTMLDocument('');
        globalThis.nodeAliasTargets = [detached.createTextNode('text'), new Event('non-node')];
    "#,
    );
    vm.eval(
        r#"
        const n = Node.prototype;
        const author = new Proxy(nodeAliases[0], aliasHandlerTemplate);
        const revoked = Proxy.revocable(nodeAliases[0], aliasHandlerTemplate);
        revoked.revoke();
        const nodeValue = Object.getOwnPropertyDescriptor(n, 'nodeValue');
        const textContent = Object.getOwnPropertyDescriptor(n, 'textContent');
        const reject = call => {
          let rejected = false;
          try { call(); } catch (error) { rejected = error instanceof TypeError; }
          assertAlias(rejected, 'invalid node identity was accepted');
        };
        for (const invalid of [author, revoked.proxy, revokedNodeAlias, nodeAliases[1]]) {
          reject(() => n.appendChild.call(document.body, invalid));
          reject(() => n.contains.call(document.body, invalid));
          reject(() => n.isSameNode.call(nodeAliasTargets[0], invalid));
          reject(() => n.isEqualNode.call(nodeAliasTargets[0], invalid));
          reject(() => n.cloneNode.call(invalid));
          reject(() => n.getRootNode.call(invalid));
          reject(() => nodeValue.set.call(invalid, 'invalid'));
          reject(() => textContent.set.call(invalid, 'invalid'));
        }
        assertAlias(nodeAliasTargets[0].data === 'text', 'invalid receiver changed backing');
        assertAlias(document.body.childNodes.length === 0, 'invalid argument changed tree');
        assertAlias(aliasTraps === 0, 'invalid identity resolution ran a trap');
    "#,
    )
    .unwrap();
}

#[test]
fn native_node_aliases_resolve_backing_without_running_author_traps() {
    let mut vm = new_parsed_test_vm(
        "https://node-native-aliases.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    vm.eval(
        r#"
        globalThis.aliasTraps = 0;
        globalThis.aliasHandlerTemplate = {
          get() { aliasTraps++; throw Error('get trap'); },
          getPrototypeOf() { aliasTraps++; throw Error('prototype trap'); }
        };
        const other = document.querySelector('#child').contentWindow;
        const detached = document.implementation.createHTMLDocument('');
        globalThis.nodeAliasTargets = [
          document, document.createElement('div'), document.createTextNode('text'),
          document.createDocumentFragment(), detached.createElement('select'),
          detached.createTextNode('detached'), other.document.createElement('div')
        ];
        globalThis.aliasOriginal = nodeAliasTargets[4];
        globalThis.aliasNonNode = new Event('node-alias');
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "nodeAliasTargets");
        let targets = global.get(scope, key.into()).unwrap();
        let targets = v8::Local::<v8::Array>::try_from(targets).unwrap();
        let key = crate::util::v8str(scope, "aliasHandlerTemplate");
        let template = global.get(scope, key.into()).unwrap();
        let template = v8::Local::<v8::Object>::try_from(template).unwrap();
        let key = crate::util::v8str(scope, "aliasNonNode");
        let non_node = global.get(scope, key.into()).unwrap();
        let non_node = v8::Local::<v8::Object>::try_from(non_node).unwrap();
        let handler = native_alias_handler(scope, template);
        let non_node = v8::Proxy::new(scope, non_node, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, non_node).unwrap();
        assert!(crate::web_api_interfaces::Event::is_instance(
            scope,
            non_node.into()
        ));
        assert!(
            crate::native_bridge::node_runtime_and_handle_from_object(scope, non_node.into())
                .is_err(),
        );
        assert!(
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                scope,
                non_node.into(),
            )
            .is_err(),
        );
        for index in 0..targets.length() {
            let target = targets.get_index(scope, index).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let expected = crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                scope, target,
            )
            .unwrap();
            let backing = moli_webapi_declare::web_api_object_target(scope, target).unwrap();
            let handler = native_alias_handler(scope, template);
            let proxy = v8::Proxy::new(scope, backing, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(
                crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                    scope,
                    proxy.into(),
                )
                .unwrap(),
                expected,
                "native alias {index}",
            );
            let author = v8::Proxy::new(scope, proxy.into(), handler).unwrap();
            assert!(
                crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                    scope,
                    author.into(),
                )
                .is_err(),
                "author wrapper {index}",
            );
            author.revoke();
            assert!(
                crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                    scope,
                    author.into(),
                )
                .is_err(),
                "revoked wrapper {index}",
            );
            if index == 4 {
                let key = crate::util::v8str(scope, "aliasBeforeAdoption");
                assert_eq!(
                    global.create_data_property(scope, key.into(), proxy.into()),
                    Some(true)
                );
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("aliasTraps").unwrap(), "0");
    assert_eq!(
        vm.eval("document.adoptNode(aliasOriginal); document.body.appendChild(aliasOriginal); aliasOriginal.ownerDocument === document && aliasOriginal.isConnected").unwrap(),
        "true",
    );
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "aliasOriginal");
        let original = global.get(scope, key.into()).unwrap();
        let original = v8::Local::<v8::Object>::try_from(original).unwrap();
        let key = crate::util::v8str(scope, "aliasBeforeAdoption");
        let alias = global.get(scope, key.into()).unwrap();
        let alias = v8::Local::<v8::Object>::try_from(alias).unwrap();
        let expected =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, original)
                .unwrap();
        assert_eq!(
            expected.0, host_ptr,
            "adoption reaches the current native host"
        );
        let host = unsafe { &*host_ptr };
        assert_eq!(
            host.dom_host().owner_document_handle(expected.1),
            Some(host.document_handle())
        );
        assert!(host.dom_host().is_connected_to_document(expected.1));
        assert_eq!(
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, alias)
                .unwrap(),
            expected,
            "an alias created before adoption follows the canonical native delegate",
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("aliasTraps").unwrap(), "0");
}
