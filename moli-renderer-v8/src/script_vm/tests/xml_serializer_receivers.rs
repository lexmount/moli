use super::*;

#[test]
fn xml_serialization_resolves_namespace_declaration_conflicts() {
    let mut vm = new_storage_html_test_vm("https://xml-namespace-conflicts.test/");
    assert_eq!(vm.eval(r#"(() => {
        const xmlns = 'http://www.w3.org/2000/xmlns/';
        const serializer = new XMLSerializer();
        const parser = new DOMParser();
        const signature = element => [element.namespaceURI, element.localName,
            Array.from(element.attributes).filter(attr => attr.namespaceURI !== xmlns &&
                attr.name !== 'xmlns' && !attr.name.startsWith('xmlns:'))
                .map(attr => [attr.namespaceURI, attr.localName, attr.value]).sort(),
            Array.from(element.children).map(signature)];
        const roots = [];
        const defaultConflict = document.createElementNS('urn:a', 'root');
        defaultConflict.setAttributeNS(xmlns, 'xmlns', 'urn:b');
        defaultConflict.append(document.createElementNS('urn:b', 'child'), document.createElementNS(null, 'empty'));
        roots.push(defaultConflict);
        const prefixConflict = document.createElementNS('urn:a', 'p:root');
        prefixConflict.setAttributeNS(xmlns, 'xmlns:p', 'urn:b');
        prefixConflict.setAttributeNS('urn:b', 'p:value', 'b');
        roots.push(prefixConflict);
        const plainPrefixConflict = document.createElementNS('urn:a', 'p:root');
        plainPrefixConflict.setAttribute('xmlns:p', 'urn:b');
        roots.push(plainPrefixConflict);
        const attributeConflict = document.createElementNS('urn:a', 'p:root');
        attributeConflict.setAttributeNS('urn:b', 'p:first', 'b');
        attributeConflict.setAttributeNS('urn:c', 'p:second', 'c');
        attributeConflict.setAttributeNS(xmlns, 'xmlns:ns1', 'urn:reserved');
        attributeConflict.setAttributeNS('urn:d', 'fourth', 'd');
        roots.push(attributeConflict);
        for (const root of roots) {
            const before = JSON.stringify(signature(root));
            const serialized = serializer.serializeToString(root);
            const parsed = parser.parseFromString(serialized, 'application/xml');
            if (parsed.getElementsByTagName('parsererror').length ||
                JSON.stringify(signature(parsed.documentElement)) !== before)
                throw Error('namespace round trip: ' + serialized);
            if (JSON.stringify(signature(root)) !== before) throw Error('serializer mutated source');
        }
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn xml_serializer_native_receivers_reject_forgery_and_author_proxies_in_the_callee_realm() {
    let mut vm = new_storage_html_test_vm("https://xml-serializer-receivers.test/");
    let result = vm.eval(r#"
        document.body.innerHTML = '<iframe></iframe>';
        const other = document.querySelector('iframe').contentWindow;
        const serializer = new XMLSerializer();
        const node = document.createElement('div');
        let traps = 0;
        const author = new Proxy(serializer, {
            get() { traps++; throw new Error('unexpected get trap'); },
            getPrototypeOf() { traps++; throw new Error('unexpected prototype trap'); }
        });
        const revoked = Proxy.revocable(serializer, {}); revoked.revoke();
        for (const realm of [window, other]) {
            const serialize = realm.XMLSerializer.prototype.serializeToString;
            if (serialize.call(serializer, node) !== serialize.call(new realm.XMLSerializer(), node))
                throw new Error('genuine cross-realm serializer');
            for (const receiver of [null, undefined, {}, Object.create(serializer),
                                    Object.create(XMLSerializer.prototype), author, revoked.proxy]) {
                let error;
                try { serialize.call(receiver, node); } catch (caught) { error = caught; }
                if (!error || Object.getPrototypeOf(error) !== realm.TypeError.prototype)
                    throw new Error('invalid receiver must throw the callee TypeError');
            }
        }
        if (traps !== 0) throw new Error('receiver validation invoked an author Proxy trap');
        'ok';
    "#).unwrap();
    assert_eq!(result, "ok");
}
