use super::*;

#[test]
fn node_get_root_options_use_shared_dictionary_conversion() {
    let mut vm = new_storage_html_test_vm("https://node-root-dictionary.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
                const assert = (value, message) => {if (!value) throw Error(message);};
                const host = document.createElement('section');
                document.body.appendChild(host);
                const shadow = host.attachShadow({mode:'open'});
                const child = document.createElement('span');
                shadow.appendChild(child);
                assert(child.getRootNode() === shadow, 'omitted options');
                for (const options of [undefined, null, {}])
                    assert(child.getRootNode(options) === shadow, 'empty dictionary');
                for (const composed of [false, undefined, null, 0, NaN, ''])
                    assert(child.getRootNode({composed}) === shadow, 'false composed');
                for (const composed of [true, 1, 'yes', {}, Symbol('truthy')])
                    assert(child.getRootNode({composed}) === document, 'true composed');
                let reads = 0;
                let conversions = 0;
                const options = new Proxy({composed:{
                    valueOf(){conversions++; throw Error('boolean conversion invoked valueOf');}
                }}, {get(target, name){reads++; assert(name === 'composed', 'dictionary member'); return target[name];}});
                assert(child.getRootNode(options) === document && reads === 1 && conversions === 0, 'one Boolean conversion');
                const marker = {};
                let caught;
                try {child.getRootNode({get composed(){throw marker;}});} catch (error) {caught=error;}
                assert(caught === marker, 'getter exception identity');
                const fn = () => {};
                fn.composed = true;
                assert(child.getRootNode(fn) === document, 'callable dictionary');
                for (const value of [false, 0, '', Symbol('options'), 1n]) {
                    let error;
                    try {child.getRootNode(value);} catch (caught) {error=caught;}
                    assert(error instanceof TypeError, 'primitive dictionary must fail');
                }
                Object.defineProperty(Object.prototype, 'composed', {configurable:true, get(){throw marker;}});
                try {
                    assert(child.getRootNode() === shadow && child.getRootNode(null) === shadow && child.getRootNode(undefined) === shadow, 'nullish dictionaries do not read Object.prototype');
                } finally {delete Object.prototype.composed;}
                const detached = document.implementation.createHTMLDocument('');
                const detachedChild = detached.createElement('p');
                detached.body.appendChild(detachedChild);
                assert(detachedChild.getRootNode({composed:true}) === detached, 'detached document root');
                document.body.appendChild(document.createElement('iframe'));
                const other = document.querySelector('iframe').contentWindow;
                const operation = other.Node.prototype.getRootNode;
                assert(operation.call(child, {composed:true}) === document, 'borrowed real node');
                let realmError;
                try {operation.call(child, 1);} catch (error) {realmError=error;}
                assert(Object.getPrototypeOf(realmError) === other.TypeError.prototype, 'callee realm TypeError');
                return 'ok';
            })()"#,
        )
        .unwrap(),
        "ok"
    );
}
