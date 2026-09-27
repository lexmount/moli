use super::*;

#[test]
fn document_location_tracks_activity_and_forwards_original_href_values() {
    let mut vm = new_parsed_test_vm(
        "https://document-location.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
        const assert = (value, label) => { if (!value) throw new Error(label); };
        const descriptor = Object.getOwnPropertyDescriptor(document, 'location');
        const get = descriptor.get, set = descriptor.set;
        const fails = action => {
            let error;
            try { action(); } catch (value) { error = value; }
            assert(error instanceof TypeError, 'expected native TypeError');
        };
        assert(get.call(document) === location, 'live Document keeps its Location');
        for (const value of [{}, Object.create(Document.prototype), document.body]) {
            fails(() => get.call(value));
            fails(() => set.call(value, 'ignored'));
        }
        const detached = document.implementation.createHTMLDocument('');
        let converted = 0;
        const href = {toString() { converted++; return '#forwarded'; }};
        assert(get.call(detached) === null, 'windowless Document has no active Location');
        fails(() => set.call(detached, href));
        assert(converted === 0, 'null forwarding fails before string conversion');
        const frame = document.createElement('iframe');
        document.body.appendChild(frame);
        const child = frame.contentDocument;
        const nested = child.createElement('iframe');
        child.body.appendChild(nested);
        const descendant = nested.contentDocument;
        assert(get.call(child) !== null && get.call(descendant) !== null, 'live descendants');
        frame.remove();
        assert(get.call(child) === null, 'removed frame Document');
        assert(get.call(descendant) === null, 'inactive ancestor makes its descendant inactive');
        fails(() => set.call(child, href));
        assert(converted === 0, 'inactive Document does not convert the value');
        set.call(document, href);
        assert(converted === 1 && location.hash === '#forwarded', 'href owns one conversion');
        return 'ok';
    })()"#
        )
        .unwrap(),
        "ok"
    );
}
