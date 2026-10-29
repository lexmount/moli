use super::*;

#[test]
fn html_element_construction_ignores_replaced_public_reflect() {
    let mut vm = new_storage_html_test_vm("https://html-constructor-reflect-intrinsic.test/");
    assert_eq!(vm.eval(r#"(() => {
        const child = document.body.appendChild(document.createElement('iframe')).contentWindow;
        for (const [index, owner] of [window, child].entries()) {
            const Constructor = owner.Function('return class InternalElement extends HTMLElement {}')();
            const name = 'x-private-reflect-' + index;
            owner.customElements.define(name, Constructor);
            let reads = 0;
            Object.defineProperty(owner, 'Reflect', {configurable: true, get() { reads++; throw Error('public Reflect'); }});
            const direct = new Constructor();
            const created = owner.document.createElement(name);
            if (reads !== 0 || !(direct instanceof Constructor) || !(created instanceof Constructor) ||
                direct.localName !== name || created.localName !== name ||
                direct.ownerDocument !== owner.document || created.ownerDocument !== owner.document)
                throw Error('internal HTML construction must use its intrinsic');
        }
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn html_constructor_proxy_handlers_ignore_inherited_traps_in_each_realm() {
    let mut vm = new_parsed_test_vm(
        "https://constructor-proxy-handler.test/",
        "<!doctype html><body></body>",
    );
    let result = vm.eval(r#"(() => {
        const frame = document.body.appendChild(document.createElement('iframe'));
        const probe = w => {
            const ctor = w.HTMLDivElement;
            const ctorPrototype = ctor.prototype;
            const parent = Object.getPrototypeOf(ctor);
            const key = Symbol('trap probe');
            const typeError = callback => { try { callback(); } catch (error) { return error instanceof w.TypeError; } return false; };
            const operations = [
                ['get', () => ctor.prototype === ctorPrototype],
                ['set', () => { const ok=Reflect.set(ctor,key,42); const value=ctor[key]; Reflect.deleteProperty(ctor,key); return ok && value===42; }],
                ['has', () => 'prototype' in ctor],
                ['getOwnPropertyDescriptor', () => Object.getOwnPropertyDescriptor(ctor,'prototype').value === ctorPrototype],
                ['defineProperty', () => { const ok=Reflect.defineProperty(ctor,key,{value:42,configurable:true}); const value=ctor[key]; Reflect.deleteProperty(ctor,key); return ok && value===42; }],
                ['deleteProperty', () => { Reflect.defineProperty(ctor,key,{value:42,configurable:true}); return Reflect.deleteProperty(ctor,key) && !Object.hasOwn(ctor,key); }],
                ['ownKeys', () => Reflect.ownKeys(ctor).includes('prototype')],
                ['getPrototypeOf', () => Object.getPrototypeOf(ctor) === parent],
                ['setPrototypeOf', () => Reflect.setPrototypeOf(ctor,parent)],
                ['isExtensible', () => Reflect.isExtensible(ctor)],
                ['apply', () => typeError(() => Reflect.apply(ctor,null,[]))],
                ['construct', () => typeError(() => Reflect.construct(ctor,[]))],
                ['preventExtensions', () => Reflect.preventExtensions(ctor) && !Reflect.isExtensible(ctor)],
            ];
            const checks=[];
            for (const [trap,operation] of operations) {
                for (const accessor of [false,true]) {
                    let calls=0;
                    const poison=()=>{calls++;throw Error('inherited '+trap);};
                    const descriptor=Object.create(null);
                    descriptor.configurable=true;
                    descriptor[accessor?'get':'value']=poison;
                    Object.defineProperty(w.Object.prototype,trap,descriptor);
                    let passed=false;
                    try { passed=operation(); } catch (_) {} finally { delete w.Object.prototype[trap]; }
                    checks.push(passed && calls===0);
                }
            }
            return checks;
        };
        try { return JSON.stringify([probe(window),probe(frame.contentWindow)]); }
        finally { frame.remove(); }
    })()"#).expect("constructor Proxy traps should be isolated");
    let checks: Vec<Vec<bool>> = serde_json::from_str(&result).unwrap();
    assert_eq!(checks, vec![vec![true; 26]; 2]);
}
