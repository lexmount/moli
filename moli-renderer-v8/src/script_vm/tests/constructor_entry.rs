use super::*;

#[test]
fn webidl_constructors_preserve_explicit_prototypes_and_conversion_order_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://constructor-entry.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("constructor_entry.js")).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__uiEventResults.failed)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "203");
}

#[test]
fn lazy_constructor_materialization_keeps_captured_intrinsics_and_public_metadata() {
    let mut vm = new_storage_page_task_executor_test_vm("https://constructor-intrinsic.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
        const reflectConstruct = Reflect.construct;
        const functionToString = Function.prototype.toString;
        Reflect.construct = () => { throw Error('author Reflect.construct'); };
        Reflect = null;
        Object.prototype.get = () => { throw Error('inherited get trap'); };
        Object.prototype.construct = () => { throw Error('inherited construct trap'); };
        const C = VideoColorSpace;
        const value = new C({matrix: 'rgb'});
        const descriptor = Object.getOwnPropertyDescriptor(C, 'prototype');
        return C.prototype.constructor === C && Object.getPrototypeOf(value) === C.prototype
            && value.matrix === 'rgb' && C.length === 0 && C.name === 'VideoColorSpace'
            && !descriptor.writable && !descriptor.enumerable && !descriptor.configurable
            && JSON.stringify(Object.getOwnPropertyNames(C)) === '["length","name","prototype"]'
            && functionToString.call(C).includes('[native code]')
            && Object.getPrototypeOf(reflectConstruct(C, [])) === C.prototype;
    })()"#
        )
        .unwrap(),
        "true"
    );
}
