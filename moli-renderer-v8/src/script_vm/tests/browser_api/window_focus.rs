use super::*;

#[test]
fn window_focus_and_blur_validate_the_receiver_in_the_callee_realm() {
    let mut vm = new_storage_test_vm("https://window-focus-native.test/");
    vm.eval(r#"(() => {
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        const frame = document.createElement('iframe');
        frame.id = 'focus-native-child';
        body.append(frame);
    })()"#).unwrap();
    materialize_single_child_default_realm_for_test(&mut vm, "Window focus child realm");
    assert_eq!(
        vm.eval(
            r#"(() => {
        const frame = document.getElementById('focus-native-child');
        const other = frame.contentWindow;
        const fake = Object.create(Object.getPrototypeOf(other));
        for (const name of ['focus', 'blur']) {
            for (const receiver of [fake, Object.create(other), new Proxy(other, {})]) {
                let error;
                try { Reflect.apply(other[name], receiver, []); } catch (caught) { error = caught; }
                if (!(error instanceof other.TypeError) || error instanceof TypeError)
                    throw Error(name + ' callee realm brand check');
            }
            if (Reflect.apply(other[name], window, []) !== undefined)
                throw Error(name + ' genuine cross-realm Window');
        }
        const focus = other.focus;
        if (focus() !== undefined || document.activeElement !== frame)
            throw Error('extracted focus must select its child browsing context');
        if (other.blur.call(null) !== undefined) throw Error('null global receiver');
        frame.remove();
        if (other.focus() !== undefined || other.blur() !== undefined)
            throw Error('retired genuine Window should be a safe no-op');
        return true;
    })()"#
        )
        .unwrap(),
        "true"
    );
}
