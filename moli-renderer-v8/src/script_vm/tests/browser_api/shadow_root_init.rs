use super::*;

#[test]
fn shadow_root_init_converts_once_before_attaching_live_or_detached_roots() {
    let mut vm = new_storage_html_test_vm("https://shadow-root-init.test/");
    assert_eq!(vm.eval(r#"(() => {
        const members = ['clonable', 'customElementRegistry', 'delegatesFocus', 'mode',
            'referenceTarget', 'serializable', 'slotAssignment'];
        const sentinel = {sentinel: true};
        for (const owner of [document, document.implementation.createHTMLDocument('detached')]) {
            for (const member of members) {
                const host = owner.createElement('div');
                owner.body.appendChild(host);
                const reads = [];
                const init = new Proxy({mode: 'open'}, {get(target, key) {
                    reads.push(key);
                    if (key === member) throw sentinel;
                    return target[key];
                }});
                let caught;
                try {host.attachShadow(init);} catch (error) {caught = error;}
                if (caught !== sentinel || host.shadowRoot !== null ||
                    reads.join(',') !== members.slice(0, members.indexOf(member) + 1).join(','))
                    throw Error('failed conversion mutated a host or read later members: ' + member);
                if (!host.attachShadow({mode:'open'})) throw Error('host was not reusable');
            }
            const host = owner.createElement('div');
            const reads = [];
            const root = host.attachShadow(new Proxy({mode:'closed', clonable:true,
                serializable:true, delegatesFocus:true, slotAssignment:'manual',
                referenceTarget:'target', customElementRegistry:null}, {get(target,key) {
                    reads.push(key); return target[key];
                }}));
            if (reads.join(',') !== members.join(',') || root.mode !== 'closed' ||
                !root.clonable || !root.serializable || !root.delegatesFocus ||
                root.slotAssignment !== 'manual' || root.referenceTarget !== 'target')
                throw Error('converted state');
            for (const init of [undefined, null, 1, true, 'open', {}, {mode:'invalid'},
                {mode:'open',slotAssignment:'invalid'}, {mode:'open',customElementRegistry:{}}]) {
                const invalidHost = owner.createElement('div');
                let error;
                try {invalidHost.attachShadow(init);} catch (caught) {error=caught;}
                if (!(error instanceof TypeError) || invalidHost.shadowRoot !== null)
                    throw Error('invalid dictionary accepted');
            }
        }
        return true;
    })()"#).unwrap(), "true");
}
