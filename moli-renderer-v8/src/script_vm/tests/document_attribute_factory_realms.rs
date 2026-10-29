use super::*;

#[test]
fn document_attribute_factory_realms() {
    let mut vm = new_parsed_test_vm(
        "https://factory-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const assert = (ok,label) => { if (!ok) throw Error(label); };
        const frame = document.body.appendChild(document.createElement('iframe'));
        const child = frame.contentWindow;

        for (const [owner,caller] of [[window,child],[child,window]]) {
            const doc = owner.document;
            const attribute = caller.Document.prototype.createAttribute.call(doc,'data-value');
            const ns = caller.Document.prototype.createAttributeNS.call(doc,'urn:test','p:value');
            for (const value of [attribute,ns]) {
                assert(Object.getPrototypeOf(value) === owner.Attr.prototype,'Attr owner prototype');
                assert(value.ownerDocument === doc,'Attr ownerDocument');
            }
            assert(ns.namespaceURI === 'urn:test' && ns.prefix === 'p','namespace data');
        }
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn detached_document_attribute_factories_allocate_in_the_document_realm() {
    let mut vm = new_parsed_test_vm(
        "https://detached-attribute-factory-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const assert = (ok, label) => { if (!ok) throw Error(label); };
        const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
        for (const [owner, caller] of [[window, child], [child, window]]) {
            const documents = [
                new owner.Document(),
                owner.document.implementation.createHTMLDocument('detached'),
                owner.document.implementation.createDocument('urn:root', 'root', null),
            ];
            for (const doc of documents) {
                let conversions = 0;
                const attribute = caller.Document.prototype.createAttribute.call(doc, {
                    toString() { conversions++; return 'DATA-X'; }
                });
                const namespaced = caller.Document.prototype.createAttributeNS.call(doc, {
                    toString() { conversions++; return 'urn:test'; }
                }, { toString() { conversions++; return 'p:value'; } });
                assert(conversions === 3, 'argument conversion count');
                for (const attr of [attribute, namespaced]) {
                    assert(attr.ownerDocument === doc, 'ownerDocument');
                    assert(Object.getPrototypeOf(attr) === owner.Attr.prototype, 'detached Attr prototype');
                    assert(attr instanceof owner.Attr && !(attr instanceof caller.Attr), 'detached Attr instanceof');
                }
                assert(attribute.name === (doc.contentType === 'text/html' ? 'data-x' : 'DATA-X'), 'name case');
                assert(namespaced.namespaceURI === 'urn:test' && namespaced.prefix === 'p' && namespaced.localName === 'value', 'namespace identity');
                let error;
                try { caller.Document.prototype.createAttribute.call(doc, 'bad name'); } catch (caught) { error = caught; }
                assert(error instanceof caller.DOMException && !(error instanceof owner.DOMException), 'validation stays in method realm');
                error = undefined;
                try { caller.Document.prototype.createAttributeNS.call(doc, null, 'p:x'); } catch (caught) { error = caught; }
                assert(error instanceof caller.DOMException && !(error instanceof owner.DOMException), 'namespace validation realm');
            }
        }
        return true;
    })()"#).unwrap(), "true");
}
