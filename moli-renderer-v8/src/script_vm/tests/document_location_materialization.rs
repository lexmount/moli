use super::*;

#[test]
fn native_document_factories_materialize_unforgeable_location_accessors() {
    let mut vm = new_parsed_test_vm(
        "https://document-location-factories.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const assert = (value, label) => { if (!value) throw new Error(label); };
        const docs = [
            new Document(),
            document.implementation.createHTMLDocument(''),
            document.implementation.createDocument(null, 'root'),
            new DOMParser().parseFromString('<root/>', 'application/xml'),
            new DOMParser().parseFromString('<p>inert', 'text/html'),
            document.cloneNode(false),
            document.cloneNode(true)
        ];
        const original = Object.getOwnPropertyDescriptor(document, 'location');
        for (const doc of docs) {
            const descriptor = Object.getOwnPropertyDescriptor(doc, 'location');
            assert(descriptor.get === original.get && descriptor.set === original.set, 'same-realm accessor identity');
            assert(!!descriptor && typeof descriptor.get === 'function' && typeof descriptor.set === 'function', 'own accessor');
            assert(descriptor.configurable === false && descriptor.enumerable === true, 'unforgeable descriptor');
            assert(descriptor.get.name === 'get location' && descriptor.set.name === 'set location', 'native names');
            assert(descriptor.get.length === 0 && descriptor.set.length === 1, 'native lengths');
            assert(doc.location === null && descriptor.get.call(doc) === null, 'windowless Location');
            let converted = 0, error;
            try { descriptor.set.call(doc, {toString() { converted++; return '#ignored'; }}); }
            catch (value) { error = value; }
            assert(error instanceof TypeError && converted === 0, 'null forwarding fails before conversion');
            assert(!Reflect.defineProperty(doc, 'location', {value: 'forged'}), 'cannot replace Location');
            assert(!Reflect.deleteProperty(doc, 'location'), 'cannot delete Location');
        }
        const frame = document.createElement('iframe');
        document.body.appendChild(frame);
        const child = frame.contentWindow;
        const doc = new child.Document();
        const descriptor = Object.getOwnPropertyDescriptor(doc, 'location');
        assert(Object.getPrototypeOf(descriptor.get) === child.Function.prototype, 'factory getter realm');
        assert(Object.getPrototypeOf(descriptor.set) === child.Function.prototype, 'factory setter realm');
        assert(doc.location === null, 'child factory stays windowless');
        let error;
        try { descriptor.set.call(doc, 'ignored'); } catch (value) { error = value; }
        assert(error instanceof child.TypeError, 'factory setter exception realm');
        frame.remove();
        return 'ok';
    })()"#).unwrap(), "ok");
}
